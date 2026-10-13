//! The den's view of each room's public MLS state (RFC 9420 §§ 12–13, what
//! a Delivery Service may hold), through OpenMLS's `PublicGroup`.
//!
//! Members send commits as PublicMessages: signed, readable, and checkable
//! here. The den keeps a room's public state (its ratchet tree and group
//! context: who is in it, by signature key) and orders a commit only if it
//! verifies against that state, naming the device that signed it; and takes
//! a GroupInfo only if it describes that same state. A member can therefore
//! no longer freeze a room with bytes shaped like a commit, nor fork its
//! newcomers into another group. Application messages stay PrivateMessages
//! the den cannot read.
//!
//! State goes in and out as JSON (the storage provider's values, hex): the
//! den keeps it per room and hands it back on the next commit.

use std::collections::HashMap;

use openmls::prelude::tls_codec::{Deserialize as _, Serialize as _};
use openmls::prelude::group_info::VerifiableGroupInfo;
use openmls::prelude::*;
use openmls_memory_storage::MemoryStorage;
use openmls_rust_crypto::RustCrypto;
use rustler::Binary;

fn e<T: std::fmt::Debug>(what: &'static str) -> impl Fn(T) -> String {
    move |x| format!("{what}: {x:?}")
}

fn load(state: &str) -> Result<MemoryStorage, String> {
    let map: HashMap<String, String> = serde_json::from_str(state).map_err(e("state"))?;
    let storage = MemoryStorage::default();
    {
        let mut values = storage.values.write().map_err(|_| "state lock".to_string())?;
        for (k, v) in map {
            values.insert(hex::decode(k).map_err(e("state key"))?, hex::decode(v).map_err(e("state value"))?);
        }
    }
    Ok(storage)
}

fn dump(storage: &MemoryStorage) -> Result<String, String> {
    let values = storage.values.read().map_err(|_| "state lock".to_string())?;
    let map: HashMap<String, String> = values.iter().map(|(k, v)| (hex::encode(k), hex::encode(v))).collect();
    serde_json::to_string(&map).map_err(e("state"))
}

fn group_info(bytes: &[u8]) -> Result<VerifiableGroupInfo, String> {
    let msg = MlsMessageIn::tls_deserialize_exact(bytes).map_err(e("group info"))?;
    match msg.extract() {
        MlsMessageBodyIn::GroupInfo(g) => Ok(g),
        _ => Err("not a GroupInfo".into()),
    }
}

/// A room's public state from a GroupInfo carrying its ratchet tree: the
/// tree is checked against the group context, the GroupInfo's signature
/// against its signer's leaf.
fn public_from(crypto: &RustCrypto, storage: &MemoryStorage, gi: VerifiableGroupInfo) -> Result<PublicGroup, String> {
    let tree = gi.extensions().ratchet_tree().ok_or("a GroupInfo without its ratchet tree")?.ratchet_tree().clone();
    PublicGroup::from_external(crypto, storage, tree, gi, ProposalStore::new()).map(|(pg, _)| pg).map_err(e("group info"))
}

fn context_bytes(pg: &PublicGroup) -> Result<Vec<u8>, String> {
    pg.group_context().tls_serialize_detached().map_err(e("group context"))
}

/// Start tracking a room from its first GroupInfo: `{state, room, epoch}`.
#[rustler::nif(schedule = "DirtyCpu")]
fn track(gi: Binary) -> Result<(String, String, u64), String> {
    let (crypto, storage) = (RustCrypto::default(), MemoryStorage::default());
    let pg = public_from(&crypto, &storage, group_info(gi.as_slice())?)?;
    let room = String::from_utf8(pg.group_context().group_id().as_slice().to_vec()).map_err(e("room id"))?;
    Ok((dump(&storage)?, room, pg.group_context().epoch().as_u64()))
}

/// A commit, checked and applied: `{state, epoch, signer key (hex)}`. The
/// signer is the member whose leaf signed it, or, for an external commit,
/// the joiner's new leaf.
#[rustler::nif(schedule = "DirtyCpu")]
fn commit(state: String, msg: Binary) -> Result<(String, u64, String), String> {
    let crypto = RustCrypto::default();
    let storage = load(&state)?;
    let m = MlsMessageIn::tls_deserialize_exact(msg.as_slice()).map_err(e("message"))?;
    let protocol: ProtocolMessage = m.try_into_protocol_message().map_err(e("message"))?;
    let gid = protocol.group_id().clone();
    let mut pg = PublicGroup::load(&storage, &gid).map_err(e("state"))?.ok_or("the den does not track that room")?;
    let processed = pg.process_message(&crypto, protocol).map_err(e("commit"))?;
    let sender = processed.sender().clone();
    let ProcessedMessageContent::StagedCommitMessage(staged) = processed.into_content() else {
        return Err("not a commit".into());
    };
    // An open room is joined by each device's own external commit: nobody adds
    // a device to it, which would take that device's seat (and its claimed
    // invitation) without it, and lock it out.
    let private = gid.as_slice().len() == 34 && gid.as_slice().starts_with(b"p-");
    if !private && staged.add_proposals().next().is_some() {
        return Err("members join an open room themselves; nobody adds them".into());
    }
    let key = match sender {
        Sender::Member(idx) => pg.members().find(|m| m.index == idx).map(|m| m.signature_key).ok_or("its signer is not in the room")?,
        Sender::NewMemberCommit => staged.update_path_leaf_node().ok_or("an external commit without its new leaf")?.signature_key().as_slice().to_vec(),
        _ => return Err("a commit from outside the room".into()),
    };
    pg.merge_commit(&storage, *staged).map_err(e("merge"))?;
    Ok((dump(&storage)?, pg.group_context().epoch().as_u64(), hex::encode(key)))
}

/// Whether a GroupInfo describes the state the den tracks (same group
/// context: epoch, tree hash, transcript), signed by a leaf of its tree.
#[rustler::nif(schedule = "DirtyCpu")]
fn info_matches(state: String, gi: Binary) -> Result<bool, String> {
    let crypto = RustCrypto::default();
    let storage = load(&state)?;
    let vgi = group_info(gi.as_slice())?;
    let gid = vgi.group_id().clone();
    let pg = PublicGroup::load(&storage, &gid).map_err(e("state"))?.ok_or("the den does not track that room")?;
    let fresh = public_from(&crypto, &MemoryStorage::default(), vgi)?;
    Ok(context_bytes(&fresh)? == context_bytes(&pg)?)
}

/// The signature keys (hex) of the room's members in the tracked state: who
/// may read and write it.
#[rustler::nif(schedule = "DirtyCpu")]
fn members(state: String, room: String) -> Result<Vec<String>, String> {
    let storage = load(&state)?;
    let gid = GroupId::from_slice(room.as_bytes());
    let pg = PublicGroup::load(&storage, &gid).map_err(e("state"))?.ok_or("the den does not track that room")?;
    Ok(pg.members().map(|m| hex::encode(m.signature_key)).collect())
}

rustler::init!("Elixir.Den.Public");
