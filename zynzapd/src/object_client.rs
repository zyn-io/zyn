//! What the wallet can learn from an object ledger.
//!
//! The object ledger answers no question about an account. It serves
//! commitments, ephemeral keys and ciphertexts — the same bytes to everyone —
//! and a holder finds their own money by trying their viewing key against all
//! of it (§133.5). That is the whole privacy argument, and it means the wallet
//! cannot ask "what do I hold": it has to download and reconstruct.
//!
//! So this module is deliberately thin. It fetches `zyn_published`, hands the
//! bytes to [`Scanner`], and returns what opened. The node is never told which
//! outputs were interesting, because it is never asked about any of them.


use zyn_vm::note_enc::{EncryptedNote, StaticSecret};
use zyn_vm::scan::{PublishedOutput, Scanner, Wallet};
use zyn_vm::spec::AccountId;

/// Why a scan could not be completed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ObjectError {
    /// The node could not be reached, or closed the connection.
    Unreachable(String),
    /// The node answered with something that is not a published feed.
    Malformed(&'static str),
}

impl std::fmt::Display for ObjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectError::Unreachable(e) => write!(f, "the object ledger is not answering: {e}"),
            ObjectError::Malformed(w) => write!(f, "the object ledger's reply is malformed: {w}"),
        }
    }
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok())
        .collect()
}

fn hash32(s: &str) -> Option<[u8; 32]> {
    unhex(s)?.try_into().ok()
}

/// Every `"key":"value"` for one key, in order.
///
/// A parser rather than a JSON dependency because the shape is fixed and
/// small, and because the wallet already carries enough.
fn values(body: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{}\":\"", key);
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find(&needle) {
        rest = &rest[at + needle.len()..];
        match rest.find('"') {
            Some(end) => {
                out.push(rest[..end].to_string());
                rest = &rest[end..];
            }
            None => break,
        }
    }
    out
}

/// The nullifier list is a bare array of strings, so it needs its own read.
fn nullifiers(body: &str) -> Vec<[u8; 32]> {
    let Some(at) = body.find("\"nullifiers\":[") else {
        return Vec::new();
    };
    let rest = &body[at + "\"nullifiers\":[".len()..];
    let end = rest.find(']').unwrap_or(0);
    rest[..end]
        .split(',')
        .filter_map(|s| hash32(s.trim().trim_matches('"')))
        .collect()
}

/// One POST, no dependencies.
/// Where the object ledger answers.
///
/// Accepts a full URL or a bare `host:port`. The bare form is what the node's
/// own loopback address looks like and is assumed to be plain HTTP on `/`;
/// anything else is taken as written, which is what lets a desktop wallet
/// reach the public endpoint — that one is HTTPS and lives on a path, and the
/// raw-socket client this replaced could express neither.
fn url_of(host: &str) -> String {
    if host.starts_with("http://") || host.starts_with("https://") {
        host.to_string()
    } else {
        format!("http://{host}/")
    }
}

fn rpc(host: &str, body: &str) -> Result<String, ObjectError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();
    let response = agent
        .post(&url_of(host))
        .set("content-type", "application/json")
        .send_string(body)
        .map_err(|e| ObjectError::Unreachable(e.to_string()))?;
    // Bounded. The feed grows with the chain and comes from somewhere this
    // wallet does not control, so it is not read without a ceiling.
    let mut out = String::new();
    std::io::Read::read_to_string(
        &mut std::io::Read::take(response.into_reader(), 64 << 20),
        &mut out,
    )
    .map_err(|e| ObjectError::Unreachable(e.to_string()))?;
    Ok(out)
}

/// Parse a published feed into what a scanner takes.
pub fn parse_published(body: &str) -> Result<(Vec<PublishedOutput>, Vec<[u8; 32]>), ObjectError> {
    let commitments = values(body, "commitment");
    let epks = values(body, "epk");
    let ciphertexts = values(body, "ciphertext");
    if commitments.len() != epks.len() || epks.len() != ciphertexts.len() {
        // A feed whose three lists disagree cannot be zipped into outputs
        // without guessing which one is authoritative, and guessing here
        // would hand the scanner a ciphertext paired with the wrong
        // commitment — which it would then correctly refuse, reporting an
        // empty wallet instead of a broken node.
        return Err(ObjectError::Malformed("the output lists disagree in length"));
    }
    let outputs = commitments
        .iter()
        .zip(epks.iter())
        .zip(ciphertexts.iter())
        .filter_map(|((c, e), x)| {
            Some(PublishedOutput {
                commitment: hash32(c)?,
                enc: EncryptedNote {
                    epk: hash32(e)?,
                    ciphertext: unhex(x)?,
                },
            })
        })
        .collect();
    Ok((outputs, nullifiers(body)))
}

/// Download what anyone could download, and find what is ours in it.
///
/// `nullifier_key` is what lets the wallet recognise its own spends. Without
/// it a balance only grows, and the wallet would offer to spend notes that
/// are already gone — so it is required rather than optional here, and
/// `Scanner::watch_only` stays the explicit way to ask for the weaker thing.
pub fn scan(
    host: &str,
    owner: &AccountId,
    viewing: StaticSecret,
    nullifier_key: [u8; 32],
) -> Result<Wallet, ObjectError> {
    let body = rpc(host, "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"zyn_published\"}")?;
    let (outputs, spent) = parse_published(&body)?;
    Ok(Scanner::new(viewing, nullifier_key).scan(owner, &outputs, 0, &spent))
}

// ---------------------------------------------------------------------------
// Acting on the ledger: the same doors the browser wallet uses.
// ---------------------------------------------------------------------------

use ed25519_dalek::{Signer, SigningKey};
use objectvm::{ObjectVm, Op, Output};
use zyn_vm::object::{nullifier, Note};
use zyn_vm::spec::MicrochainVm;
use zyn_vm::Fixed;

/// A decimal amount as WAD, exactly: at most 18 decimals, above zero.
pub fn parse_amount(text: &str) -> Result<Fixed, String> {
    let t = text.trim();
    let (whole, frac) = t.split_once('.').unwrap_or((t, ""));
    if t.ends_with('.') || whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) || frac.len() > 18 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return Err("use a positive decimal with at most 18 decimal places".into());
    }
    let w: i128 = whole.parse().map_err(|_| "amount too large")?;
    let f: i128 = if frac.is_empty() { 0 } else { format!("{frac:0<18}").parse().map_err(|_| "bad decimals")? };
    let v = w.checked_mul(zyn_vm::fixed::WAD).and_then(|x| x.checked_add(f)).ok_or("amount too large")?;
    if v <= 0 {
        return Err("amount must be above zero".into());
    }
    Ok(Fixed(v))
}

/// One JSON-RPC call with named fields, as the object node reads them.
pub fn call(host: &str, method: &str, fields: serde_json::Value) -> Result<serde_json::Value, String> {
    let mut body = serde_json::json!({ "jsonrpc": "2.0", "id": "nap", "method": method, "params": [] });
    if let (Some(b), Some(f)) = (body.as_object_mut(), fields.as_object()) {
        for (k, v) in f {
            b.insert(k.clone(), v.clone());
        }
    }
    let text = rpc(host, &body.to_string()).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| "the object ledger's reply is not JSON".to_string())?;
    if let Some(e) = v.get("error") {
        return Err(e.get("message").and_then(|m| m.as_str()).unwrap_or("the chain refused").to_string());
    }
    Ok(v.get("result").cloned().unwrap_or(serde_json::Value::Null))
}

fn hexs(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

/// Sign `intent` with `key` and hand it to the holders' door. A refusal
/// inside the receipts is an error: the frame was taken, the intent was not.
pub fn submit(host: &str, chain: u32, key: &SigningKey, intent: Op) -> Result<String, String> {
    let status = call(host, "zyn_status", serde_json::json!({}))?;
    if status.get("chain").and_then(|c| c.as_u64()) != Some(chain as u64) {
        return Err(format!("the ledger at {host} is not chain {chain}"));
    }
    let epoch = status.get("epoch").and_then(|e| e.as_u64()).ok_or("the ledger gave no epoch")?;
    let auth = zyn_vm::auth::Authorization {
        chain_id: chain,
        vm_id: zyn_vm::zvm::vm_id::<ObjectVm>(),
        valid_until_epoch: epoch + 100,
    };
    let signature = key.sign(&zyn_vm::auth::signing_payload::<ObjectVm>(&auth, &intent));
    let cred = zyn_vm::verify::Credential::Ed25519 {
        key: key.verifying_key().to_bytes(),
        signature: signature.to_bytes(),
    };
    let genesis = <ObjectVm as MicrochainVm>::genesis(chain, objectvm::Limits { max_io: 16 });
    let authorized = zyn_vm::verify::authorize_intent::<ObjectVm>(&[cred], &auth, intent, &genesis)
        .map_err(|e| format!("cannot sign that: {e:?}"))?;
    let frame = zyn_vm::verify::encode_authorized::<ObjectVm>(&authorized);
    let r = call(host, "zyn_submitSigned", serde_json::json!({ "frame": hexs(&frame) }))?;
    let receipts = r.get("receipts").and_then(|x| x.as_str()).unwrap_or("").to_string();
    if let Some(at) = receipts.find("Refused(\"") {
        let rest = &receipts[at + 9..];
        return Err(format!("the chain refused: {}", &rest[..rest.find('"').unwrap_or(rest.len())]));
    }
    Ok(receipts)
}

fn random32() -> [u8; 32] {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b
}

/// A note for `owner`, sealed to the viewing key they registered.
pub fn sealed_output(owner: AccountId, to_viewing: &[u8; 32], asset: [u8; 32], amount: Fixed) -> Result<Output, String> {
    let note = Note { asset, amount, owner, nonce: random32() };
    let mut plain = Vec::with_capacity(80);
    plain.extend_from_slice(&note.asset);
    plain.extend_from_slice(&note.amount.0.to_be_bytes());
    plain.extend_from_slice(&note.nonce);
    let enc = zyn_vm::note_enc::seal(to_viewing, StaticSecret::from(random32()), &plain)
        .map_err(|e| format!("cannot seal the note: {e:?}"))?;
    Ok(Output { note, enc })
}

/// Notes of `asset` covering `amount`, largest first, and change back to
/// `owner` sealed to `own_viewing`.
pub fn fund(
    wallet: &Wallet,
    nullifier_key: &[u8; 32],
    owner: AccountId,
    own_viewing: &[u8; 32],
    asset: [u8; 32],
    amount: Fixed,
) -> Result<(Vec<([u8; 32], [u8; 32])>, Vec<Output>), String> {
    let mut mine: Vec<_> = wallet.live.iter().filter(|o| o.note.asset == asset).collect();
    mine.sort_by(|a, b| b.note.amount.cmp(&a.note.amount));
    let mut sum: i128 = 0;
    let mut inputs = Vec::new();
    for o in mine {
        if sum >= amount.0 {
            break;
        }
        inputs.push((o.commitment, nullifier(nullifier_key, &o.commitment)));
        sum += o.note.amount.0;
    }
    if sum < amount.0 {
        return Err("not enough of that asset".into());
    }
    if inputs.len() > 16 {
        return Err("that needs too many notes at once".into());
    }
    let change = if sum > amount.0 {
        vec![sealed_output(owner, own_viewing, asset, Fixed(sum - amount.0))?]
    } else {
        Vec::new()
    };
    Ok((inputs, change))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Amounts are exact: a wallet that rounds what someone sends is lying.
    #[test]
    fn amounts_parse_exactly_and_refuse_the_rest() {
        assert_eq!(parse_amount("1").unwrap(), Fixed::whole(1));
        assert_eq!(parse_amount("0.000000000000000001").unwrap(), Fixed(1));
        assert_eq!(parse_amount("12.5").unwrap(), Fixed(12_500_000_000_000_000_000));
        for bad in ["", "0", "-1", "1.", ".5", "1e3", "0.0000000000000000001", "1,5"] {
            assert!(parse_amount(bad).is_err(), "{bad} must be refused");
        }
    }
    use zyn_vm::note_enc::{seal, viewing_public};
    use zyn_vm::object::{commitment, nullifier, Note};
    use zyn_vm::Fixed;

    const ME: AccountId = [0xAA; 32];
    const ZEC: [u8; 32] = [0x7a; 32];

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{:02x}", x)).collect()
    }

    fn feed(entries: &[(String, String, String)], spent: &[[u8; 32]]) -> String {
        let outs: Vec<String> = entries
            .iter()
            .map(|(c, e, x)| format!("{{\"commitment\":\"{c}\",\"epk\":\"{e}\",\"ciphertext\":\"{x}\"}}"))
            .collect();
        let nulls: Vec<String> = spent.iter().map(|n| format!("\"{}\"", hex(n))).collect();
        format!(
            "{{\"result\":{{\"outputs\":[{}],\"nullifiers\":[{}]}}}}",
            outs.join(","),
            nulls.join(",")
        )
    }

    fn published(to: &StaticSecret, amount: i64, nonce: u8, eph: u8) -> (String, String, String, Note) {
        let note = Note {
            asset: ZEC,
            amount: Fixed::whole(amount),
            owner: ME,
            nonce: [nonce; 32],
        };
        let mut plain = Vec::new();
        plain.extend_from_slice(&note.asset);
        plain.extend_from_slice(&note.amount.0.to_be_bytes());
        plain.extend_from_slice(&note.nonce);
        let enc = seal(&viewing_public(to), StaticSecret::from([eph; 32]), &plain).unwrap();
        (
            hex(&commitment(&note).unwrap()),
            hex(&enc.epk),
            hex(&enc.ciphertext),
            note,
        )
    }

    /// The whole point: the wallet reconstructs its balance from a feed the
    /// node serves identically to everyone.
    #[test]
    fn a_wallet_finds_its_own_notes_in_a_public_feed() {
        let me = StaticSecret::from([0x11; 32]);
        let them = StaticSecret::from([0x22; 32]);
        let (c1, e1, x1, _) = published(&me, 5, 1, 41);
        let (c2, e2, x2, _) = published(&them, 99, 2, 42);
        let (c3, e3, x3, _) = published(&me, 12, 3, 43);
        let body = feed(&[(c1, e1, x1), (c2, e2, x2), (c3, e3, x3)], &[]);

        let (outputs, spent) = parse_published(&body).expect("parses");
        assert_eq!(outputs.len(), 3, "all three are downloaded");
        let w = Scanner::new(me, [0x31; 32]).scan(&ME, &outputs, 0, &spent);
        assert_eq!(w.live.len(), 2, "two of the three opened");
        assert_eq!(w.balance(&ZEC), Fixed::whole(17).0);
    }

    /// A spent note stops counting, which is the half a wallet cannot skip.
    #[test]
    fn a_spent_note_is_recognised() {
        let me = StaticSecret::from([0x11; 32]);
        let key = [0x31u8; 32];
        let (c1, e1, x1, n1) = published(&me, 5, 1, 41);
        let (c2, e2, x2, _) = published(&me, 12, 3, 43);
        let spent = nullifier(&key, &commitment(&n1).unwrap());
        let body = feed(&[(c1, e1, x1), (c2, e2, x2)], &[spent]);

        let (outputs, sp) = parse_published(&body).expect("parses");
        let w = Scanner::new(me, key).scan(&ME, &outputs, 0, &sp);
        assert_eq!(w.balance(&ZEC), Fixed::whole(12).0);
        assert_eq!(w.spent.len(), 1);
    }

    /// A feed whose lists disagree is refused rather than zipped. Guessing
    /// would pair a ciphertext with the wrong commitment, the scanner would
    /// correctly reject it, and the wallet would report an empty balance for
    /// what is really a broken node.
    #[test]
    fn a_feed_with_mismatched_lists_is_refused_not_guessed() {
        let me = StaticSecret::from([0x11; 32]);
        let (c1, e1, x1, _) = published(&me, 5, 1, 41);
        // Two commitments and two ephemeral keys, one ciphertext: the lists
        // genuinely disagree, which is the case that must not be zipped.
        let body = format!(
            "{{\"result\":{{\"outputs\":[\
             {{\"commitment\":\"{c1}\",\"epk\":\"{e1}\",\"ciphertext\":\"{x1}\"}},\
             {{\"commitment\":\"{c1}\",\"epk\":\"{e1}\"}}\
             ],\"nullifiers\":[]}}}}"
        );
        assert!(matches!(
            parse_published(&body),
            Err(ObjectError::Malformed(_))
        ));
    }

    /// An empty ledger is an empty wallet, not an error.
    #[test]
    fn an_empty_feed_is_an_empty_wallet() {
        let (outputs, spent) =
            parse_published("{\"result\":{\"outputs\":[],\"nullifiers\":[]}}").expect("parses");
        assert!(outputs.is_empty() && spent.is_empty());
        let w = Scanner::new(StaticSecret::from([0x11; 32]), [0x31; 32]).scan(&ME, &outputs, 0, &spent);
        assert_eq!(w.balance(&ZEC), 0);
    }

    /// An unreachable node is reported as such rather than as an empty
    /// wallet. "Nothing" and "we could not ask" look identical and mean
    /// opposite things.
    #[test]
    fn an_unreachable_node_is_not_an_empty_wallet() {
        let err = scan(
            "127.0.0.1:1",
            &ME,
            StaticSecret::from([0x11; 32]),
            [0x31; 32],
        )
        .expect_err("nothing listens on port 1");
        assert!(matches!(err, ObjectError::Unreachable(_)));
        assert!(err.to_string().contains("not answering"));
    }
}
