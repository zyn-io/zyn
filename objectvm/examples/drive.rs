//! Emit an intent as hex, for driving a devnet node by hand.
fn main() {
    use objectvm::{ObjectVm, Op};
    use zyn_vm::note_enc::{seal, viewing_public, StaticSecret};
    use zyn_vm::object::Note;
    use zyn_vm::spec::MicrochainVm;
    use zyn_vm::Fixed;

    let which: u8 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(1);
    let zec = [0x7au8; 32];
    let alice = [0xAAu8; 32];
    let view = StaticSecret::from([0xAAu8; 32]);
    let note = Note { asset: zec, amount: Fixed::whole(20), owner: alice, nonce: [0x40 + which; 32] };
    let mut plain = Vec::new();
    plain.extend_from_slice(&note.asset);
    plain.extend_from_slice(&note.amount.0.to_be_bytes());
    plain.extend_from_slice(&note.nonce);
    let enc = seal(&viewing_public(&view), StaticSecret::from([0x40 + which; 32]), &plain).unwrap();

    let op = Op::Deposit {
        program: zyn_vm::program::deposit(),
        asset: zec, amount: Fixed::whole(20), owner: alice,
        nonce: [0x40 + which; 32], enc,
        external_ref: [0xE0 + which; 32],
    };
    println!("{}", ObjectVm::encode_intent(&op).iter().map(|b| format!("{:02x}", b)).collect::<String>());
}
