//! Which exits on an object ledger can be paid, and to where.
//!
//! An object exit names a **commitment**, not an address: `H(address ‖ salt)`.
//! The chain never carries where anyone's money went, so the operator cannot
//! pay an exit until its holder reveals the preimage out of band. That is the
//! privacy property and it is also the reason this file exists — matching a
//! reveal to an exit is a check, and a check that lives inside a settler loop
//! is a check nothing tests.
//!
//! So the decision is separated from the payment: this module says what may be
//! paid and what is blocked, and the settler does the Zcash work. The AMM's
//! `ZcashSettler` answers the same question against `SwapState`, where an exit
//! belongs to an account and is paid to that account's bound address. Neither
//! half of that is true here, which is why this is a different module rather
//! than a generic parameter on that one.

use std::collections::BTreeMap;

use objectvm::ObjectVm;
use zyn_custody::payout::Destination;
use zyn_vm::commit::Hash;
use zyn_vm::Fixed;

/// Zatoshi, in `Fixed`'s units.
const ZAT: i128 = 10_000_000_000;

/// A holder saying where their exit should be paid.
///
/// Proof, not instruction: it is accepted only when it reproduces the
/// commitment the chain already committed to. A reveal naming an address the
/// exit never named is not a redirection, it is a stranger.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reveal {
    pub address: Destination,
    pub salt: [u8; 32],
}

/// One exit that can be paid now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Payable {
    /// Index into the ledger's append-only `exits`, which is what
    /// `Op::SettleExit` names and what makes settlement checkable.
    pub index: u32,
    pub zatoshi: u64,
    pub to: Destination,
}

/// An exit that cannot be paid yet, and why.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blocked {
    /// Nobody has said where to pay it. Normal, and not an error: the holder
    /// reveals when they are ready, and nothing expires.
    AwaitingReveal { index: u32 },
    /// The amount is not a whole number of zatoshi. Zcash cannot pay a
    /// fraction of one, and rounding somebody's exit is not this code's
    /// decision to make.
    NotWholeZatoshi { index: u32 },
    /// The amount is zero or negative. A chain that produced this is wrong,
    /// and paying it would move value the wrong way.
    NotPositive { index: u32 },
}

/// Split the ledger's unpaid exits into what can be paid and what cannot.
///
/// `reveals` is matched by recomputing the commitment, so the map is keyed by
/// what the chain committed to rather than by anything the revealer chose.
pub fn payable(
    vm: &ObjectVm,
    asset: &Hash,
    reveals: &[Reveal],
) -> (Vec<Payable>, Vec<Blocked>) {
    payable_from(&vm.exits, asset, reveals)
}

/// [`payable`] over the ledger's exits alone, so a caller can copy them out
/// under the node lock and decide — and pay — after letting go of it (A162).
pub fn payable_from(
    exits: &[objectvm::PendingExit],
    asset: &Hash,
    reveals: &[Reveal],
) -> (Vec<Payable>, Vec<Blocked>) {
    // Keyed by the commitment the reveal *reproduces*, not by anything it
    // claims. A reveal that matches no exit simply never matches.
    let by_commitment: BTreeMap<Hash, Destination> = reveals
        .iter()
        .map(|r| {
            (
                crate::settle::zcash_commitment(&r.address, &r.salt),
                r.address,
            )
        })
        .collect();

    let mut ready = Vec::new();
    let mut blocked = Vec::new();
    for (i, exit) in exits.iter().enumerate() {
        if exit.paid_by.is_some() || exit.asset != *asset {
            continue;
        }
        let index = i as u32;
        if exit.amount.0 <= 0 {
            blocked.push(Blocked::NotPositive { index });
            continue;
        }
        if exit.amount.0 % ZAT != 0 {
            blocked.push(Blocked::NotWholeZatoshi { index });
            continue;
        }
        match by_commitment.get(&exit.destination) {
            None => blocked.push(Blocked::AwaitingReveal { index }),
            Some(to) => ready.push(Payable {
                index,
                zatoshi: (exit.amount.0 / ZAT) as u64,
                to: *to,
            }),
        }
    }
    (ready, blocked)
}

/// What the ledger still owes, in zatoshi, whether or not it can be paid.
///
/// The figure that decides whether the vault can cover its exits at all. It
/// counts blocked exits too: an exit nobody has revealed is still a debt, and
/// a vault sized only for the revealed ones is a vault that runs out.
pub fn owed(vm: &ObjectVm, asset: &Hash) -> u64 {
    vm.exits
        .iter()
        .filter(|e| e.paid_by.is_none() && e.asset == *asset && e.amount.0 > 0)
        .map(|e| (e.amount.0 / ZAT) as u64)
        .sum()
}

/// The intent that records a payment on the ledger.
pub fn settled(index: u32, txid: Hash) -> objectvm::Op {
    objectvm::Op::SettleExit {
        program: zyn_vm::program::settle_exit(),
        index,
        txid,
    }
}

/// `Fixed` for a whole number of zatoshi.
pub fn from_zatoshi(z: u64) -> Fixed {
    Fixed(z as i128 * ZAT)
}


// ---------------------------------------------------------------------------
// Paying them.
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};

use orchard::keys::{FullViewingKey, Scope};
use orchard::ValuePool;
use zyn_custody::payout::{self, Envelope, Payment};
use zyn_custody::shielded::PoolStores;
use orchard::bundle::BundleVersion;
use zcash_protocol::consensus::Network;
use zyn_custody::zebra::Zebra;

/// A payment that has been broadcast and is waiting for confirmations.
///
/// Persisted, because the window between "Zcash has the transaction" and "the
/// ledger has recorded it" is exactly where a restart loses money: the vault
/// has paid, the chain still says it owes, and a settler with no memory pays
/// again.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InFlight {
    pub txid: String,
    /// Ledger indices this transaction paid.
    pub covers: Vec<u32>,
}

/// `<txid> <index>,<index>,…` per line.
fn encode_inflight(v: &[InFlight]) -> String {
    v.iter()
        .map(|f| {
            format!(
                "{} {}\n",
                f.txid,
                f.covers
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect()
}

fn decode_inflight(s: &str) -> Vec<InFlight> {
    s.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let txid = f.next()?.to_string();
            let covers = f
                .next()?
                .split(',')
                .filter(|x| !x.is_empty())
                .map(|x| x.parse().ok())
                .collect::<Option<Vec<u32>>>()?;
            Some(InFlight { txid, covers })
        })
        .collect()
}

/// Pays an object ledger's exits out of the vault.
///
/// Returns the intents that record what Zcash has confirmed rather than
/// applying them itself: every intent this node accepts goes through one
/// journal path, and a settler writing its own record would be a second copy
/// of that path's accounting.
pub struct ExitSettler {
    zebra: Zebra,
    network: Network,
    public: zyn_custody::custody_net::ZcashPublicPackage,
    custodians: Vec<String>,
    threshold: u16,
    fvk: FullViewingKey,
    notes: PoolStores,
    reveals: Vec<Reveal>,
    asset: Hash,
    confirmations: u64,
    inflight: Vec<InFlight>,
    path: PathBuf,
    proving: Option<(BundleVersion, orchard::circuit::ProvingKey)>,
    warned: bool,
}

impl ExitSettler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zebra: Zebra,
        network: Network,
        public: zyn_custody::custody_net::ZcashPublicPackage,
        custodians: Vec<String>,
        threshold: u16,
        fvk: FullViewingKey,
        notes: PoolStores,
        reveals: Vec<Reveal>,
        asset: Hash,
        confirmations: u64,
        dir: &Path,
        chain_id: u32,
    ) -> ExitSettler {
        let path = dir.join(format!("chain-{chain_id}.exits-inflight"));
        let inflight = std::fs::read_to_string(&path)
            .ok()
            .map(|t| decode_inflight(&t))
            .unwrap_or_default();
        if !inflight.is_empty() {
            eprintln!(
                "zyn-object-node: {} exit payment(s) still confirming from a previous run",
                inflight.len()
            );
        }
        ExitSettler {
            zebra,
            network,
            public,
            custodians,
            threshold,
            fvk,
            notes,
            reveals,
            asset,
            confirmations,
            inflight,
            path,
            proving: None,
            warned: false,
        }
    }

    pub fn add_reveal(&mut self, r: Reveal) {
        if !self.reveals.contains(&r) {
            self.reveals.push(r);
        }
    }

    fn save(&self) {
        let _ = std::fs::write(&self.path, encode_inflight(&self.inflight));
    }

    /// One pass. Returns the intents to apply.
    /// `exits` is a copy of the ledger's, taken under the node lock and
    /// passed after it is released: paying reads the vault's note stores,
    /// which the note scan can hold for minutes (A162).
    pub fn poll_once(&mut self, exits: &[objectvm::PendingExit]) -> Result<Vec<objectvm::Op>, String> {
        let settled = self.follow_up()?;
        // One payment in flight at a time. The vault's notes are the input to
        // the next transaction, and building a second one against notes the
        // first already spent produces a transaction Zcash refuses.
        if self.inflight.is_empty() {
            self.pay_next(exits)?;
        }
        Ok(settled)
    }

    /// Record what Zcash has buried deep enough to be believed.
    fn follow_up(&mut self) -> Result<Vec<objectvm::Op>, String> {
        let mut out = Vec::new();
        let mut done = Vec::new();
        for (i, f) in self.inflight.iter().enumerate() {
            match self.zebra.confirmations(&f.txid).map_err(|e| e.to_string())? {
                Some(depth) if depth >= self.confirmations => {
                    let Some(txid) = unhex32(&f.txid) else {
                        return Err(format!("broadcast txid {} is not 32 hex bytes", f.txid));
                    };
                    for index in &f.covers {
                        out.push(settled(*index, txid));
                    }
                    eprintln!(
                        "zyn-object-node: exit payment {} confirmed at depth {depth}, {} exit(s)",
                        f.txid,
                        f.covers.len()
                    );
                    done.push(i);
                }
                // Still shallow, or not in the chain yet. Neither is an error.
                _ => {}
            }
        }
        for i in done.into_iter().rev() {
            self.inflight.remove(i);
        }
        if !out.is_empty() {
            self.save();
        }
        Ok(out)
    }

    /// Build, sign and broadcast one payment covering what can be paid.
    fn pay_next(&mut self, exits: &[objectvm::PendingExit]) -> Result<(), String> {
        let (ready, blocked) = payable_from(exits, &self.asset, &self.reveals);
        if !blocked.is_empty() && !self.warned {
            eprintln!(
                "zyn-object-node: {} exit(s) cannot be paid yet: {:?}",
                blocked.len(),
                &blocked[..blocked.len().min(4)]
            );
            self.warned = true;
        }
        if ready.is_empty() {
            return Ok(());
        }

        let tip = self.zebra.block_count().map_err(|e| e.to_string())?;
        let env = Envelope::at(self.network, tip as u32);
        let change_to = self.fvk.address_at(0u32, Scope::External);
        let ovk = Some(self.fvk.to_ovk(Scope::External));

        // Ironwood first: it is the only pool that can pay a shielded address
        // (§17), so a shielded exit waits for Ironwood funds rather than being
        // silently dropped from an Orchard attempt.
        let mut attempt = None;
        for pool in [ValuePool::Ironwood, ValuePool::Orchard] {
            let Ok(version) = env.bundle_version_for(pool) else {
                continue;
            };
            let (mut pay, cov): (Vec<Payment>, Vec<u32>) =
                if version.default_flags().cross_address_enabled() {
                    (
                        ready.iter().map(|r| Payment::new(r.to, r.zatoshi)).collect(),
                        ready.iter().map(|r| r.index).collect(),
                    )
                } else {
                    ready
                        .iter()
                        .filter(|r| matches!(r.to, Destination::Transparent(_)))
                        .map(|r| (Payment::new(r.to, r.zatoshi), r.index))
                        .unzip()
                };
            if pay.is_empty() {
                continue;
            }
            let store = self
                .notes
                .of(pool)
                .lock()
                .map_err(|_| "note store poisoned".to_string())?;
            // Build once to learn the fee, then take it from the first payout
            // so what leaves the vault equals what the ledger burned. Charging
            // it to the vault instead would drift the balance below issuance,
            // and the next solvency check would refuse.
            let probe = match payout::build(
                &store, &self.fvk, ovk.clone(), &pay, change_to, version, rand::rngs::OsRng,
            ) {
                Ok(p) => p,
                Err(payout::PayoutError::Notes(_)) => continue,
                Err(e) => return Err(format!("cannot build payout: {e}")),
            };
            if pay[0].zatoshi <= probe.fee {
                return Err("the first exit cannot cover the network fee".into());
            }
            pay[0].zatoshi -= probe.fee;
            match payout::build(
                &store, &self.fvk, ovk.clone(), &pay, change_to, version, rand::rngs::OsRng,
            ) {
                Ok(p) => {
                    attempt = Some((p, cov, version));
                    break;
                }
                Err(payout::PayoutError::Notes(_)) => continue,
                Err(e) => return Err(format!("cannot build payout: {e}")),
            }
        }
        let Some((mut built, covers, version)) = attempt else {
            eprintln!(
                "zyn-object-node: no pool can cover {} exit(s) yet; waiting",
                ready.len()
            );
            return Ok(());
        };

        let sighash = built.sighash(&env).map_err(|e| e.to_string())?;
        built
            .finalize_io(sighash, rand::rngs::OsRng)
            .map_err(|e| e.to_string())?;
        if self.proving.as_ref().map(|(v, _)| *v != version).unwrap_or(true) {
            eprintln!("zyn-object-node: building the Orchard proving key for exits");
            self.proving = Some((version, payout::proving_key(version)));
        }
        built
            .prove(&self.proving.as_ref().unwrap().1, rand::rngs::OsRng)
            .map_err(|e| e.to_string())?;

        let group = *self.public.verifying_key();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut q = zyn_custody::custody_net::RemoteQuorum::new(self.custodians.clone());
        payout::sign_all_with(
            &mut built, sighash, &mut q, self.threshold, &group, &self.public, now,
        )
        .map_err(|e| e.to_string())?;

        let fee = built.fee;
        let sealed = built
            .extract(sighash, &env, rand::rngs::OsRng)
            .map_err(|e| e.to_string())?;
        let txid = self
            .zebra
            .send_raw_transaction(&sealed.hex())
            .map_err(|e| e.to_string())?;

        // Written down **before** it is reported. A broadcast this node
        // forgets is a payment it will make twice.
        self.inflight.push(InFlight {
            txid: txid.clone(),
            covers: covers.clone(),
        });
        self.save();
        eprintln!(
            "zyn-object-node: exit payment {txid} broadcast from {:?}, {} exit(s), fee {fee} zat",
            version.circuit_version(),
            covers.len()
        );
        Ok(())
    }
}

fn unhex_bytes(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok())
        .collect()
}

fn unhex32(s: &str) -> Option<Hash> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}


/// Load the reveals an operator has been given, out of band.
///
/// `<address> <salt-hex>` per line, `#` comments allowed. No account column,
/// unlike the AMM's file: an object exit belongs to a commitment, not to an
/// account, and asking for an account here would invite writing down a link
/// the chain deliberately does not carry.
///
/// A line that does not parse is an error rather than a skip. A reveal the
/// operator believes they filed and that was silently dropped is an exit that
/// waits forever for a reason nobody can see.
pub fn load_reveals(path: &Path, network: Network) -> Result<Vec<Reveal>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 2 {
            return Err(format!("line {}: expected `<address> <salt-hex>`", n + 1));
        }
        let address = crate::settle::parse_destination(f[0], network).ok_or_else(|| {
            format!(
                "line {}: not a transparent or unified address for this network",
                n + 1
            )
        })?;
        let salt: [u8; 32] = unhex_bytes(f[1])
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| format!("line {}: salt must be 64 hex characters", n + 1))?;
        out.push(Reveal { address, salt });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use objectvm::{Limits, PendingExit};
    use zcash_transparent::address::TransparentAddress;
    use zyn_vm::spec::MicrochainVm;

    const ZEC: Hash = [0x7a; 32];
    const OTHER: Hash = [0xC0; 32];

    fn to(h: u8) -> Destination {
        Destination::Transparent(TransparentAddress::PublicKeyHash([h; 20]))
    }

    fn reveal(h: u8, salt: u8) -> Reveal {
        Reveal {
            address: to(h),
            salt: [salt; 32],
        }
    }

    fn vm_with(exits: Vec<PendingExit>) -> ObjectVm {
        let mut vm = ObjectVm::genesis(212, Limits { max_io: 8 });
        vm.exits = exits;
        vm
    }

    fn pending(asset: Hash, whole: i64, dest: Hash) -> PendingExit {
        PendingExit {
            asset,
            amount: Fixed(whole as i128 * ZAT),
            destination: dest,
            since_epoch: 1,
            paid_by: None,
        }
    }

    /// The ordinary path: a holder reveals the preimage, the exit becomes
    /// payable, and the amount converts to whole zatoshi.
    #[test]
    fn a_revealed_exit_is_payable() {
        let r = reveal(0xAA, 0x01);
        let commitment = crate::settle::zcash_commitment(&r.address, &r.salt);
        let vm = vm_with(vec![pending(ZEC, 3, commitment)]);
        let (ready, blocked) = payable(&vm, &ZEC, &[r]);
        assert_eq!(blocked, Vec::new());
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].index, 0);
        assert_eq!(ready[0].zatoshi, 3);
        assert_eq!(ready[0].to, to(0xAA));
    }

    /// The security property. A reveal is a proof, not an instruction: one
    /// that does not reproduce the chain's commitment must not redirect a
    /// payment, or anyone who can post a reveal can take anyone's exit.
    #[test]
    fn a_reveal_that_does_not_reproduce_the_commitment_pays_nobody() {
        let honest = reveal(0xAA, 0x01);
        let commitment = crate::settle::zcash_commitment(&honest.address, &honest.salt);
        let vm = vm_with(vec![pending(ZEC, 3, commitment)]);

        // A stranger's address, and the honest salt.
        let thief = reveal(0xBB, 0x01);
        let (ready, blocked) = payable(&vm, &ZEC, &[thief]);
        assert_eq!(ready, Vec::new(), "a forged reveal must pay nothing");
        assert_eq!(blocked, vec![Blocked::AwaitingReveal { index: 0 }]);

        // The right address with the wrong salt is equally not a match.
        let wrong_salt = reveal(0xAA, 0x02);
        let (ready, _) = payable(&vm, &ZEC, &[wrong_salt]);
        assert_eq!(ready, Vec::new());
    }

    /// An exit nobody has revealed is waiting, not broken. Nothing expires it.
    #[test]
    fn an_unrevealed_exit_waits() {
        let vm = vm_with(vec![pending(ZEC, 1, [0xFF; 32])]);
        let (ready, blocked) = payable(&vm, &ZEC, &[]);
        assert!(ready.is_empty());
        assert_eq!(blocked, vec![Blocked::AwaitingReveal { index: 0 }]);
    }

    /// Already paid, so it is not paid again — the check that stops a restart
    /// from double-spending the vault.
    #[test]
    fn an_exit_already_paid_is_not_payable_again() {
        let r = reveal(0xAA, 0x01);
        let c = crate::settle::zcash_commitment(&r.address, &r.salt);
        let mut e = pending(ZEC, 3, c);
        e.paid_by = Some([0xD1; 32]);
        let vm = vm_with(vec![e]);
        let (ready, blocked) = payable(&vm, &ZEC, &[r]);
        assert!(ready.is_empty() && blocked.is_empty());
    }

    /// Another asset's exit is not this settler's to pay.
    #[test]
    fn an_exit_in_another_asset_is_left_alone() {
        let r = reveal(0xAA, 0x01);
        let c = crate::settle::zcash_commitment(&r.address, &r.salt);
        let vm = vm_with(vec![pending(OTHER, 3, c)]);
        let (ready, blocked) = payable(&vm, &ZEC, &[r]);
        assert!(ready.is_empty() && blocked.is_empty());
    }

    /// Zcash cannot pay a fraction of a zatoshi, and rounding somebody's exit
    /// is not this code's decision.
    #[test]
    fn a_fractional_amount_is_blocked_not_rounded() {
        let r = reveal(0xAA, 0x01);
        let c = crate::settle::zcash_commitment(&r.address, &r.salt);
        let mut e = pending(ZEC, 3, c);
        e.amount = Fixed(e.amount.0 + 1); // one unit under a whole zatoshi
        let vm = vm_with(vec![e]);
        let (ready, blocked) = payable(&vm, &ZEC, &[r]);
        assert!(ready.is_empty());
        assert_eq!(blocked, vec![Blocked::NotWholeZatoshi { index: 0 }]);
    }

    /// A non-positive exit would move value the wrong way.
    #[test]
    fn a_non_positive_exit_is_refused() {
        let r = reveal(0xAA, 0x01);
        let c = crate::settle::zcash_commitment(&r.address, &r.salt);
        let mut e = pending(ZEC, 0, c);
        e.amount = Fixed(-ZAT);
        let vm = vm_with(vec![e]);
        let (_, blocked) = payable(&vm, &ZEC, &[r]);
        assert_eq!(blocked, vec![Blocked::NotPositive { index: 0 }]);
    }

    /// The index must be the ledger's own, because that is what `SettleExit`
    /// names — an index counted over the filtered list would settle the wrong
    /// exit the moment one is skipped.
    #[test]
    fn the_index_is_the_ledgers_not_the_filtered_lists() {
        let r = reveal(0xAA, 0x01);
        let c = crate::settle::zcash_commitment(&r.address, &r.salt);
        let mut paid = pending(ZEC, 1, c);
        paid.paid_by = Some([0xD1; 32]);
        // index 0 paid, index 1 another asset, index 2 the payable one.
        let vm = vm_with(vec![paid, pending(OTHER, 1, c), pending(ZEC, 5, c)]);
        let (ready, _) = payable(&vm, &ZEC, &[r]);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].index, 2, "the ledger's index, not the filtered one");
    }

    /// What the vault must be able to cover counts unrevealed exits too: an
    /// exit nobody has revealed is still a debt.
    #[test]
    fn owed_counts_exits_nobody_has_revealed() {
        let vm = vm_with(vec![
            pending(ZEC, 3, [0x01; 32]),
            pending(ZEC, 4, [0x02; 32]),
        ]);
        assert_eq!(owed(&vm, &ZEC), 7);
    }
}

#[cfg(test)]
mod inflight_tests {
    use super::*;

    /// The window a restart loses money in: Zcash has the transaction, the
    /// ledger does not yet say so. A settler that forgot would pay again.
    #[test]
    fn the_in_flight_record_round_trips() {
        let v = vec![
            InFlight {
                txid: "ab".repeat(32),
                covers: vec![0, 3, 7],
            },
            InFlight {
                txid: "cd".repeat(32),
                covers: vec![11],
            },
        ];
        assert_eq!(decode_inflight(&encode_inflight(&v)), v);
    }

    /// A truncated or corrupt line is dropped rather than guessed: a half-read
    /// index list would settle the wrong exit.
    #[test]
    fn a_malformed_line_is_dropped_not_guessed() {
        assert!(decode_inflight("notatxid\n").is_empty());
        assert!(decode_inflight("ab 1,two,3\n").is_empty());
        // A record with no covers at all is not a payment worth remembering.
        assert!(decode_inflight("ab\n").is_empty());
    }

    /// An empty list is a valid state, not a parse failure — it is what a
    /// settler with nothing outstanding writes.
    #[test]
    fn nothing_in_flight_round_trips_as_nothing() {
        assert!(decode_inflight(&encode_inflight(&[])).is_empty());
    }

    /// A txid that is not 32 bytes must not become a settlement: the ledger
    /// would record a payment referencing a transaction that cannot exist.
    #[test]
    fn a_txid_that_is_not_32_bytes_is_refused() {
        assert!(unhex32("abc").is_none());
        assert!(unhex32(&"zz".repeat(32)).is_none());
        assert_eq!(unhex32(&"ab".repeat(32)), Some([0xAB; 32]));
    }
}

#[cfg(test)]
mod reveal_file_tests {
    use super::*;
    use zcash_protocol::consensus::Network;

    fn write(text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("zyn-reveals-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(format!("{}.reveals", text.len()));
        std::fs::write(&p, text).unwrap();
        p
    }

    /// A transparent testnet address and a salt, with comments and blanks.
    #[test]
    fn a_reveal_file_parses() {
        let p = write(concat!(
            "# where to pay exit 0\n",
            "tm9iMLAuYMzJ6jtFLcA7rzUmfreGuKvr7Ma 0101010101010101010101010101010101010101010101010101010101010101\n\n"
        ));
        let r = load_reveals(&p, Network::TestNetwork).expect("parses");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].salt, [0x01; 32]);
    }

    /// A line that does not parse is an error, not a skip. A reveal the
    /// operator believes they filed and that was silently dropped is an exit
    /// that waits forever for a reason nobody can see.
    #[test]
    fn a_malformed_line_is_an_error_not_a_silent_skip() {
        assert!(load_reveals(&write("only-one-field\n"), Network::TestNetwork).is_err());
        assert!(load_reveals(&write("notanaddress 0101\n"), Network::TestNetwork).is_err());
        // Right address, salt too short.
        assert!(load_reveals(
            &write("tm9iMLAuYMzJ6jtFLcA7rzUmfreGuKvr7Ma 0101\n"),
            Network::TestNetwork
        )
        .is_err());
    }

    /// A mainnet address is not a testnet reveal. Paying one would send a
    /// testnet exit to an address on the wrong chain.
    #[test]
    fn an_address_for_the_wrong_network_is_refused() {
        assert!(load_reveals(
            &write(concat!("t1KkTCS5wWLBhUwsmYtmoLnuAJUL8rEfKtL 0101010101010101010101010101010101010101010101010101010101010101\n")),
            Network::TestNetwork
        )
        .is_err());
    }

    /// An empty file is a valid state: no reveals filed yet.
    #[test]
    fn an_empty_file_is_no_reveals() {
        assert!(load_reveals(&write("# nothing yet\n"), Network::TestNetwork)
            .unwrap()
            .is_empty());
    }
}
