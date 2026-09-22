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

use std::io::{Read, Write};
use std::net::TcpStream;

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
fn rpc(host: &str, body: &str) -> Result<String, ObjectError> {
    let mut s = TcpStream::connect(host).map_err(|e| ObjectError::Unreachable(e.to_string()))?;
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(20)));
    let _ = s.set_write_timeout(Some(std::time::Duration::from_secs(20)));
    let req = format!(
        "POST / HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        host,
        body.len(),
        body
    );
    s.write_all(req.as_bytes())
        .map_err(|e| ObjectError::Unreachable(e.to_string()))?;
    let mut out = String::new();
    s.read_to_string(&mut out)
        .map_err(|e| ObjectError::Unreachable(e.to_string()))?;
    let at = out
        .find("\r\n\r\n")
        .ok_or(ObjectError::Malformed("no body"))?;
    Ok(out[at + 4..].to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
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
