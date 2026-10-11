//! The Cave den's end-to-end encryption (DECISIONS §184).
//!
//! * **Rooms are MLS groups** (RFC 9420, OpenMLS). A member's device holds an
//!   Ed25519 signature key and a pseudonym that names nothing; it creates a
//!   room or joins one by an external commit against the room's published
//!   GroupInfo (ratchet tree included), sends and reads application
//!   messages, and removes members whose day passes ended. The server only
//!   relays and orders ciphertext: every commit a member makes and every
//!   message is a PrivateMessage; only external joins are public, as MLS
//!   requires.
//! * **Entry tokens are RFC 9474 blind signatures** (RSABSSA-SHA384-PSS-
//!   Randomized): the device blinds a token, the door signs it after checking
//!   a member pass, the device unblinds it, and presents it later from
//!   another connection. The door cannot link the two.
//!
//! The core here is plain Rust, tested natively; `wasm` wraps it for the page.

use std::collections::HashMap;

use openmls::prelude::tls_codec::{Deserialize as TlsDeserialize, Serialize as TlsSerialize};
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::OpenMlsProvider;
use serde::{Deserialize, Serialize};

/// X25519, AES-128-GCM, SHA-256, Ed25519: MLS's mandatory suite.
pub const SUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

fn err<E: std::fmt::Debug>(what: &str) -> impl FnOnce(E) -> String + '_ {
    move |e| format!("{what}: {e:?}")
}

/// Commits a member makes are encrypted; incoming may be either, since an
/// external join is always public.
fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .wire_format_policy(MIXED_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .use_ratchet_tree_extension(true)
        .padding_size(64)
        .max_past_epochs(4)
        .build()
}

/// What a processed message was, for the page.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Event {
    /// Words from a member.
    Text { from: String, key: String, text: String, epoch: u64 },
    /// The room changed: who is in it now.
    Commit { epoch: u64, members: Vec<Seat> },
    /// A proposal, kept for the next commit.
    Proposal { epoch: u64 },
    /// This device's own message, relayed back: already shown.
    Echo { epoch: u64 },
}

/// One member of a room: their pseudonym and their signature key.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Seat {
    pub name: String,
    pub key: String,
}

/// What a device keeps between visits.
#[derive(Serialize, Deserialize)]
struct Saved {
    name: String,
    key: String,
    groups: Vec<String>,
    values: Vec<(String, String)>,
}

pub struct Member {
    provider: OpenMlsRustCrypto,
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
    name: String,
    groups: HashMap<String, MlsGroup>,
}

impl Member {
    /// A new device: a fresh signature key and a pseudonym from it, naming
    /// neither the wallet nor the item.
    pub fn new() -> Result<Member, String> {
        let provider = OpenMlsRustCrypto::default();
        let signer = SignatureKeyPair::new(SUITE.signature_algorithm()).map_err(err("key"))?;
        signer.store(provider.storage()).map_err(err("store key"))?;
        let name = format!("m-{}", &hex::encode(signer.to_public_vec())[..10]);
        let credential = CredentialWithKey {
            credential: BasicCredential::new(name.clone().into_bytes()).into(),
            signature_key: signer.to_public_vec().into(),
        };
        Ok(Member { provider, signer, credential, name, groups: HashMap::new() })
    }

    /// A device as it was saved.
    pub fn restore(saved: &str) -> Result<Member, String> {
        let s: Saved = serde_json::from_str(saved).map_err(err("saved state"))?;
        let provider = OpenMlsRustCrypto::default();
        {
            let mut values = provider.storage().values.write().map_err(|_| "storage lock".to_string())?;
            for (k, v) in &s.values {
                values.insert(hex::decode(k).map_err(err("saved key"))?, hex::decode(v).map_err(err("saved value"))?);
            }
        }
        let key = hex::decode(&s.key).map_err(err("saved signature key"))?;
        let signer = SignatureKeyPair::read(provider.storage(), &key, SUITE.signature_algorithm()).ok_or("the saved state has no signing key")?;
        let credential = CredentialWithKey {
            credential: BasicCredential::new(s.name.clone().into_bytes()).into(),
            signature_key: key.into(),
        };
        let mut groups = HashMap::new();
        for g in &s.groups {
            let id = GroupId::from_slice(g.as_bytes());
            if let Some(group) = MlsGroup::load(provider.storage(), &id).map_err(err("load room"))? {
                groups.insert(g.clone(), group);
            }
        }
        Ok(Member { provider, signer, credential, name: s.name, groups })
    }

    pub fn save(&self) -> Result<String, String> {
        let values = self.provider.storage().values.read().map_err(|_| "storage lock".to_string())?;
        let saved = Saved {
            name: self.name.clone(),
            key: self.key(),
            groups: self.groups.keys().cloned().collect(),
            values: values.iter().map(|(k, v)| (hex::encode(k), hex::encode(v))).collect(),
        };
        serde_json::to_string(&saved).map_err(err("save"))
    }

    pub fn name(&self) -> String {
        self.name.clone()
    }

    /// The signature key, hex: what the server binds a session to.
    pub fn key(&self) -> String {
        hex::encode(self.signer.to_public_vec())
    }

    pub fn rooms(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }

    /// Start a room. Returns its GroupInfo (tree included), for the server.
    pub fn create(&mut self, room: &str) -> Result<Vec<u8>, String> {
        let group = MlsGroup::builder()
            .ciphersuite(SUITE)
            .with_group_id(GroupId::from_slice(room.as_bytes()))
            .with_wire_format_policy(MIXED_CIPHERTEXT_WIRE_FORMAT_POLICY)
            .use_ratchet_tree_extension(true)
            .padding_size(64)
            .max_past_epochs(4)
            .build(&self.provider, &self.signer, self.credential.clone())
            .map_err(err("create room"))?;
        self.groups.insert(room.to_string(), group);
        self.group_info(room)
    }

    /// The room's GroupInfo now, for whoever joins next.
    pub fn group_info(&mut self, room: &str) -> Result<Vec<u8>, String> {
        let (crypto, signer) = (self.provider.crypto(), &self.signer);
        let group = self.groups.get(room).ok_or_else(|| format!("not in {room}"))?;
        let info = group.export_group_info(crypto, signer, true).map_err(err("group info"))?;
        info.tls_serialize_detached().map_err(err("group info bytes"))
    }

    /// Join a room by an external commit against its GroupInfo. The returned
    /// commit goes to the server; if it refuses (someone committed first),
    /// `forget` the room and join again from the newer GroupInfo.
    pub fn join(&mut self, room: &str, group_info: &[u8]) -> Result<Vec<u8>, String> {
        let msg = MlsMessageIn::tls_deserialize_exact(group_info).map_err(err("group info"))?;
        let MlsMessageBodyIn::GroupInfo(info) = msg.extract() else { return Err("that is not a GroupInfo".into()) };
        let (group, bundle) = MlsGroup::external_commit_builder()
            .with_config(join_config())
            .build_group(&self.provider, info, self.credential.clone())
            .map_err(err("join"))?
            .load_psks(self.provider.storage())
            .map_err(err("join psks"))?
            .build(self.provider.rand(), self.provider.crypto(), &self.signer, |_| true)
            .map_err(err("join commit"))?
            .finalize(&self.provider)
            .map_err(err("join finalize"))?;
        if group.group_id().as_slice() != room.as_bytes() {
            return Err("that GroupInfo is for another room".into());
        }
        self.groups.insert(room.to_string(), group);
        bundle.into_commit().tls_serialize_detached().map_err(err("commit bytes"))
    }

    /// Drop a room this device is in (after a refused join, or on leaving).
    pub fn forget(&mut self, room: &str) -> Result<(), String> {
        if let Some(mut g) = self.groups.remove(room) {
            g.delete(self.provider.storage()).map_err(err("forget"))?;
        }
        Ok(())
    }

    /// One-time invitations to this device (KeyPackages), for the den to hand
    /// out to whoever adds it to a private room.
    pub fn key_packages(&mut self, n: usize) -> Result<Vec<Vec<u8>>, String> {
        (0..n)
            .map(|_| {
                let bundle = KeyPackage::builder().build(SUITE, &self.provider, &self.signer, self.credential.clone()).map_err(err("key package"))?;
                bundle.key_package().tls_serialize_detached().map_err(err("key package bytes"))
            })
            .collect()
    }

    /// Add devices to a private room by their KeyPackages: a commit for the
    /// room (pending until the den orders it) and a Welcome for the new ones.
    pub fn add(&mut self, room: &str, key_packages: &[Vec<u8>]) -> Result<(Vec<u8>, Vec<u8>), String> {
        let (provider, signer) = (&self.provider, &self.signer);
        let mut kps = Vec::new();
        for b in key_packages {
            let kp = KeyPackageIn::tls_deserialize_exact(b).map_err(err("key package"))?;
            kps.push(kp.validate(provider.crypto(), ProtocolVersion::Mls10).map_err(err("key package check"))?);
        }
        let group = self.groups.get_mut(room).ok_or_else(|| format!("not in {room}"))?;
        let (commit, welcome, _info) = group.add_members(provider, signer, &kps).map_err(err("add"))?;
        Ok((commit.tls_serialize_detached().map_err(err("commit bytes"))?, welcome.tls_serialize_detached().map_err(err("welcome bytes"))?))
    }

    /// Join a private room from the Welcome someone sent this device.
    /// Returns the room.
    pub fn accept(&mut self, welcome: &[u8]) -> Result<String, String> {
        let msg = MlsMessageIn::tls_deserialize_exact(welcome).map_err(err("welcome"))?;
        let MlsMessageBodyIn::Welcome(w) = msg.extract() else { return Err("that is not a Welcome".into()) };
        let group = StagedWelcome::new_from_welcome(&self.provider, &join_config(), w, None)
            .map_err(err("welcome"))?
            .into_group(&self.provider)
            .map_err(err("join from welcome"))?;
        let room = String::from_utf8(group.group_id().as_slice().to_vec()).map_err(err("room id"))?;
        self.groups.insert(room.clone(), group);
        Ok(room)
    }

    /// A secret every member of the room derives alike for this epoch, and
    /// nobody else: the voice room's media key comes from it.
    pub fn export(&self, room: &str, label: &str, len: usize) -> Result<Vec<u8>, String> {
        let group = self.groups.get(room).ok_or_else(|| format!("not in {room}"))?;
        group.export_secret(self.provider.crypto(), label, room.as_bytes(), len).map_err(err("export"))
    }

    /// Words for a room, encrypted.
    pub fn send(&mut self, room: &str, text: &str) -> Result<Vec<u8>, String> {
        let (provider, signer) = (&self.provider, &self.signer);
        let group = self.groups.get_mut(room).ok_or_else(|| format!("not in {room}"))?;
        let out = group.create_message(provider, signer, text.as_bytes()).map_err(err("send"))?;
        out.tls_serialize_detached().map_err(err("message bytes"))
    }

    /// Take members out (by signature key) in one commit, pending until the
    /// server orders it; then `confirm`, or `discard` if it was refused.
    pub fn remove(&mut self, room: &str, keys: &[String]) -> Result<Option<Vec<u8>>, String> {
        let (provider, signer) = (&self.provider, &self.signer);
        let group = self.groups.get_mut(room).ok_or_else(|| format!("not in {room}"))?;
        let leaves: Vec<LeafNodeIndex> = group
            .members()
            .filter(|m| keys.contains(&hex::encode(&m.signature_key)))
            .map(|m| m.index)
            .collect();
        if leaves.is_empty() {
            return Ok(None);
        }
        let (commit, _welcome, _info) = group.remove_members(provider, signer, &leaves).map_err(err("remove"))?;
        Ok(Some(commit.tls_serialize_detached().map_err(err("commit bytes"))?))
    }

    /// The server ordered this device's pending commit: apply it.
    pub fn confirm(&mut self, room: &str) -> Result<u64, String> {
        let provider = &self.provider;
        let group = self.groups.get_mut(room).ok_or_else(|| format!("not in {room}"))?;
        if group.pending_commit().is_some() {
            group.merge_pending_commit(provider).map_err(err("confirm"))?;
        }
        Ok(group.epoch().as_u64())
    }

    /// The server refused this device's pending commit: drop it.
    pub fn discard(&mut self, room: &str) -> Result<(), String> {
        let provider = &self.provider;
        let group = self.groups.get_mut(room).ok_or_else(|| format!("not in {room}"))?;
        group.clear_pending_commit(provider.storage()).map_err(err("discard"))
    }

    /// One message the server relayed, read.
    pub fn receive(&mut self, room: &str, bytes: &[u8]) -> Result<Event, String> {
        let provider = &self.provider;
        let group = self.groups.get_mut(room).ok_or_else(|| format!("not in {room}"))?;
        let msg = MlsMessageIn::tls_deserialize_exact(bytes).map_err(err("message"))?;
        let protocol = msg.try_into_protocol_message().map_err(err("not a room message"))?;
        let processed = group.process_message(provider, protocol).map_err(err("read"))?;
        let sender = processed.credential().clone();
        let epoch = processed.epoch().as_u64();
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(app) => {
                let from = BasicCredential::try_from(sender).map(|c| String::from_utf8_lossy(c.identity()).to_string()).unwrap_or_default();
                let key = group
                    .members()
                    .find(|m| BasicCredential::try_from(m.credential.clone()).map(|c| String::from_utf8_lossy(c.identity()) == from).unwrap_or(false))
                    .map(|m| hex::encode(&m.signature_key))
                    .unwrap_or_default();
                Ok(Event::Text { from, key, text: String::from_utf8(app.into_bytes()).map_err(err("text"))?, epoch })
            }
            ProcessedMessageContent::StagedCommitMessage(staged) => {
                group.merge_staged_commit(provider, *staged).map_err(err("merge"))?;
                Ok(Event::Commit { epoch: group.epoch().as_u64(), members: seats(group) })
            }
            ProcessedMessageContent::ProposalMessage(p) => {
                group.store_pending_proposal(provider.storage(), *p).map_err(err("proposal"))?;
                Ok(Event::Proposal { epoch })
            }
            ProcessedMessageContent::ExternalJoinProposalMessage(p) => {
                group.store_pending_proposal(provider.storage(), *p).map_err(err("proposal"))?;
                Ok(Event::Proposal { epoch })
            }
            // The server relayed this device's own commit: that is its order.
            ProcessedMessageContent::OwnPendingCommit => {
                group.merge_pending_commit(provider).map_err(err("merge own"))?;
                Ok(Event::Commit { epoch: group.epoch().as_u64(), members: seats(group) })
            }
            ProcessedMessageContent::OwnPrivateMessage => Ok(Event::Echo { epoch }),
        }
    }

    pub fn epoch(&self, room: &str) -> Result<u64, String> {
        Ok(self.groups.get(room).ok_or_else(|| format!("not in {room}"))?.epoch().as_u64())
    }

    pub fn members(&self, room: &str) -> Result<Vec<Seat>, String> {
        Ok(seats(self.groups.get(room).ok_or_else(|| format!("not in {room}"))?))
    }
}

fn seats(group: &MlsGroup) -> Vec<Seat> {
    group
        .members()
        .map(|m| Seat {
            name: BasicCredential::try_from(m.credential.clone()).map(|c| String::from_utf8_lossy(c.identity()).to_string()).unwrap_or_default(),
            key: hex::encode(&m.signature_key),
        })
        .collect()
}

/// What the server can read from a relayed message without any key: the
/// room, the epoch, and whether it is a commit — what ordering needs.
#[derive(Serialize, Debug, PartialEq)]
pub struct Envelope {
    pub room: String,
    pub epoch: u64,
    pub commit: bool,
    pub external: bool,
}

pub fn envelope(bytes: &[u8]) -> Result<Envelope, String> {
    let msg = MlsMessageIn::tls_deserialize_exact(bytes).map_err(err("message"))?;
    let p = msg.try_into_protocol_message().map_err(err("not a room message"))?;
    let room = String::from_utf8(p.group_id().as_slice().to_vec()).map_err(err("room id"))?;
    let commit = p.content_type() == ContentType::Commit;
    let external = matches!(&p, ProtocolMessage::PublicMessage(m) if matches!(m.sender(), Sender::NewMemberCommit));
    Ok(Envelope { room, epoch: p.epoch().as_u64(), commit, external })
}

// ---------------------------------------------------------------------------
// Entry tokens: RFC 9474 blind RSA (RSABSSA-SHA384-PSS-Randomized).

pub mod token {
    use blind_rsa_signatures::{BlindMessage, BlindSignature, BlindingResult, DefaultRng, MessageRandomizer, PublicKey, Randomized, Secret, Sha384, PSS};
    use serde::{Deserialize, Serialize};

    type Pk = PublicKey<Sha384, PSS, Randomized>;

    fn err<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> String + '_ {
        move |e| format!("{what}: {e}")
    }

    /// What the device keeps between asking the door and redeeming.
    #[derive(Serialize, Deserialize)]
    pub struct Pending {
        pub msg: String,
        pub secret: String,
        pub randomizer: String,
        pub blinded: String,
    }

    /// A token for `day`, bound to the device's signature key: blinded for the
    /// door. The door sees only `blinded`.
    pub fn blind(pk_der: &[u8], day: &str, device_key: &str) -> Result<Pending, String> {
        let pk = Pk::from_der(pk_der).map_err(err("door key"))?;
        let mut nonce = [0u8; 16];
        getrandom::getrandom(&mut nonce).map_err(err("randomness"))?;
        let msg = format!("cave-den-token-v1|{day}|{device_key}|{}", hex::encode(nonce));
        let r = pk.blind(&mut DefaultRng, msg.as_bytes()).map_err(err("blind"))?;
        Ok(Pending {
            msg,
            secret: hex::encode(&r.secret.0),
            randomizer: r.msg_randomizer.map(|m| hex::encode(m.0)).unwrap_or_default(),
            blinded: hex::encode(&r.blind_message.0),
        })
    }

    /// The door's blind signature, unblinded into a signature on the token.
    pub fn finalize(pk_der: &[u8], pending: &Pending, blind_sig: &[u8]) -> Result<String, String> {
        let pk = Pk::from_der(pk_der).map_err(err("door key"))?;
        let randomizer: [u8; 32] = hex::decode(&pending.randomizer).map_err(err("randomizer"))?.try_into().map_err(|_| "randomizer length".to_string())?;
        let result = BlindingResult {
            blind_message: BlindMessage(hex::decode(&pending.blinded).map_err(err("blinded"))?),
            secret: Secret(hex::decode(&pending.secret).map_err(err("secret"))?),
            msg_randomizer: Some(MessageRandomizer(randomizer)),
        };
        let sig = pk.finalize(&BlindSignature(blind_sig.to_vec()), &result, pending.msg.as_bytes()).map_err(err("finalize"))?;
        Ok(hex::encode(&sig.0))
    }
}

// ---------------------------------------------------------------------------
// The page's interface.

#[cfg(target_arch = "wasm32")]
mod wasm {
    use wasm_bindgen::prelude::*;

    fn js(e: String) -> JsError {
        JsError::new(&e)
    }

    #[wasm_bindgen]
    pub struct Den(super::Member);

    #[wasm_bindgen]
    impl Den {
        #[wasm_bindgen(constructor)]
        pub fn new() -> Result<Den, JsError> {
            super::Member::new().map(Den).map_err(js)
        }
        pub fn restore(saved: &str) -> Result<Den, JsError> {
            super::Member::restore(saved).map(Den).map_err(js)
        }
        pub fn save(&self) -> Result<String, JsError> {
            self.0.save().map_err(js)
        }
        pub fn name(&self) -> String {
            self.0.name()
        }
        pub fn key(&self) -> String {
            self.0.key()
        }
        pub fn rooms(&self) -> Vec<String> {
            self.0.rooms()
        }
        pub fn create(&mut self, room: &str) -> Result<Vec<u8>, JsError> {
            self.0.create(room).map_err(js)
        }
        pub fn group_info(&mut self, room: &str) -> Result<Vec<u8>, JsError> {
            self.0.group_info(room).map_err(js)
        }
        pub fn join(&mut self, room: &str, group_info: &[u8]) -> Result<Vec<u8>, JsError> {
            self.0.join(room, group_info).map_err(js)
        }
        pub fn forget(&mut self, room: &str) -> Result<(), JsError> {
            self.0.forget(room).map_err(js)
        }
        pub fn send(&mut self, room: &str, text: &str) -> Result<Vec<u8>, JsError> {
            self.0.send(room, text).map_err(js)
        }
        /// Keys as a JSON array of hex strings; `undefined` when none is in the room.
        pub fn remove(&mut self, room: &str, keys: &str) -> Result<Option<Vec<u8>>, JsError> {
            let keys: Vec<String> = serde_json::from_str(keys).map_err(|e| JsError::new(&e.to_string()))?;
            self.0.remove(room, &keys).map_err(js)
        }
        /// `n` invitations to this device, as a JSON array of base64 strings.
        pub fn key_packages(&mut self, n: usize) -> Result<String, JsError> {
            use base64::Engine;
            let kps = self.0.key_packages(n).map_err(js)?;
            let enc: Vec<String> = kps.iter().map(|k| base64::engine::general_purpose::STANDARD.encode(k)).collect();
            serde_json::to_string(&enc).map_err(|e| JsError::new(&e.to_string()))
        }
        /// Key packages as a JSON array of base64; `{commit, welcome}` (base64) as JSON.
        pub fn add(&mut self, room: &str, key_packages: &str) -> Result<String, JsError> {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD;
            let kps: Vec<String> = serde_json::from_str(key_packages).map_err(|e| JsError::new(&e.to_string()))?;
            let kps: Result<Vec<Vec<u8>>, _> = kps.iter().map(|k| b64.decode(k)).collect();
            let (commit, welcome) = self.0.add(room, &kps.map_err(|e| JsError::new(&e.to_string()))?).map_err(js)?;
            Ok(serde_json::json!({ "commit": b64.encode(commit), "welcome": b64.encode(welcome) }).to_string())
        }
        pub fn accept(&mut self, welcome: &[u8]) -> Result<String, JsError> {
            self.0.accept(welcome).map_err(js)
        }
        pub fn export(&self, room: &str, label: &str, len: usize) -> Result<Vec<u8>, JsError> {
            self.0.export(room, label, len).map_err(js)
        }
        pub fn confirm(&mut self, room: &str) -> Result<u64, JsError> {
            self.0.confirm(room).map_err(js)
        }
        pub fn discard(&mut self, room: &str) -> Result<(), JsError> {
            self.0.discard(room).map_err(js)
        }
        /// The event, as JSON.
        pub fn receive(&mut self, room: &str, bytes: &[u8]) -> Result<String, JsError> {
            let e = self.0.receive(room, bytes).map_err(js)?;
            serde_json::to_string(&e).map_err(|e| JsError::new(&e.to_string()))
        }
        pub fn epoch(&self, room: &str) -> Result<u64, JsError> {
            self.0.epoch(room).map_err(js)
        }
        /// The members, as JSON.
        pub fn members(&self, room: &str) -> Result<String, JsError> {
            let m = self.0.members(room).map_err(js)?;
            serde_json::to_string(&m).map_err(|e| JsError::new(&e.to_string()))
        }
    }

    /// A blinded token for `day`, as JSON (keep it; send only `blinded`).
    #[wasm_bindgen]
    pub fn token_blind(door_key_der: &[u8], day: &str, device_key: &str) -> Result<String, JsError> {
        let p = super::token::blind(door_key_der, day, device_key).map_err(js)?;
        serde_json::to_string(&p).map_err(|e| JsError::new(&e.to_string()))
    }

    /// The door's answer, unblinded: the token's signature, hex.
    #[wasm_bindgen]
    pub fn token_finalize(door_key_der: &[u8], pending: &str, blind_sig: &[u8]) -> Result<String, JsError> {
        let p: super::token::Pending = serde_json::from_str(pending).map_err(|e| JsError::new(&e.to_string()))?;
        super::token::finalize(door_key_der, &p, blind_sig).map_err(js)
    }

    /// What the server can see of a message (for tests of the relay).
    #[wasm_bindgen]
    pub fn envelope(bytes: &[u8]) -> Result<String, JsError> {
        let e = super::envelope(bytes).map_err(js)?;
        serde_json::to_string(&e).map_err(|e| JsError::new(&e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(e: Event) -> String {
        match e {
            Event::Text { text, .. } => text,
            other => panic!("expected words, got {other:?}"),
        }
    }

    /// Three devices in one room: created, joined by external commits, words
    /// both ways, a member taken out who then reads nothing, and a device
    /// restored from its saved state that still reads.
    #[test]
    fn a_room_from_creation_to_removal() {
        let (mut a, mut b, mut c) = (Member::new().unwrap(), Member::new().unwrap(), Member::new().unwrap());
        assert!(a.name().starts_with("m-") && a.name() != b.name());

        // A starts the room; B joins from its GroupInfo; A applies B's commit.
        let info = a.create("general").unwrap();
        let join_b = b.join("general", &info).unwrap();
        let env = envelope(&join_b).unwrap();
        assert_eq!((env.room.as_str(), env.epoch, env.commit, env.external), ("general", 0, true, true));
        assert!(matches!(a.receive("general", &join_b).unwrap(), Event::Commit { epoch: 1, .. }));

        // C joins from the newer GroupInfo (any member publishes it).
        let join_c = c.join("general", &b.group_info("general").unwrap()).unwrap();
        a.receive("general", &join_c).unwrap();
        b.receive("general", &join_c).unwrap();
        assert_eq!(a.members("general").unwrap().len(), 3);
        assert_eq!(a.epoch("general").unwrap(), 2);

        // Words: the server sees only the room and the epoch.
        let m = a.send("general", "hello, den").unwrap();
        let env = envelope(&m).unwrap();
        assert_eq!((env.epoch, env.commit), (2, false));
        assert!(!String::from_utf8_lossy(&m).contains("hello, den"), "no plaintext on the wire");
        assert_eq!(text(b.receive("general", &m).unwrap()), "hello, den");
        match c.receive("general", &m).unwrap() {
            Event::Text { from, key, .. } => assert_eq!((from, key), (a.name(), a.key())),
            other => panic!("{other:?}"),
        }

        // B takes C out (C's day pass ended); the server orders it; A applies it.
        let commit = b.remove("general", &[c.key()]).unwrap().expect("C is in the room");
        assert!(envelope(&commit).unwrap().commit && !envelope(&commit).unwrap().external, "a member's commit is encrypted");
        assert_eq!(b.confirm("general").unwrap(), 3);
        a.receive("general", &commit).unwrap();
        assert_eq!(a.members("general").unwrap().len(), 2);
        let after = a.send("general", "C is gone").unwrap();
        assert!(c.receive("general", &after).is_err(), "a removed member reads nothing new");

        // A device restored from its saved state is the same member.
        let saved = b.save().unwrap();
        let mut b2 = Member::restore(&saved).unwrap();
        assert_eq!((b2.name(), b2.key()), (b.name(), b.key()));
        assert_eq!(text(b2.receive("general", &after).unwrap()), "C is gone");
        let back = b2.send("general", "restored").unwrap();
        assert_eq!(text(a.receive("general", &back).unwrap()), "restored");
    }

    /// Two members commit in the same epoch; the server keeps the first, the
    /// second discards its own and catches up.
    #[test]
    fn a_refused_commit_is_discarded_and_the_member_catches_up() {
        let (mut a, mut b, mut c) = (Member::new().unwrap(), Member::new().unwrap(), Member::new().unwrap());
        let info = a.create("lanterns").unwrap();
        let jb = b.join("lanterns", &info).unwrap();
        a.receive("lanterns", &jb).unwrap();
        let jc = c.join("lanterns", &a.group_info("lanterns").unwrap()).unwrap();
        a.receive("lanterns", &jc).unwrap();
        b.receive("lanterns", &jc).unwrap();
        // Both A and B try to take C out at epoch 2; the server orders A's.
        let ca = a.remove("lanterns", &[c.key()]).unwrap().unwrap();
        let _cb = b.remove("lanterns", &[c.key()]).unwrap().unwrap();
        a.confirm("lanterns").unwrap();
        b.discard("lanterns").unwrap();
        b.receive("lanterns", &ca).unwrap();
        assert_eq!(a.epoch("lanterns").unwrap(), b.epoch("lanterns").unwrap());
        let m = b.send("lanterns", "in step").unwrap();
        assert_eq!(text(a.receive("lanterns", &m).unwrap()), "in step");
    }

    /// A private room: invitations from the den's KeyPackage store, an Add
    /// and a Welcome, words both ways, and the same voice key on both sides.
    #[test]
    fn a_private_room_by_invitation() {
        let (mut a, mut b, mut c) = (Member::new().unwrap(), Member::new().unwrap(), Member::new().unwrap());
        let kps_b = b.key_packages(2).unwrap();
        let _ = a.create("dm-7f3a").unwrap();
        let (commit, welcome) = a.add("dm-7f3a", &kps_b[..1]).unwrap();
        assert!(envelope(&commit).unwrap().commit && !envelope(&commit).unwrap().external);
        a.confirm("dm-7f3a").unwrap();
        assert_eq!(b.accept(&welcome).unwrap(), "dm-7f3a");
        assert_eq!(a.members("dm-7f3a").unwrap().len(), 2);
        let m = b.send("dm-7f3a", "just us").unwrap();
        assert_eq!(text(a.receive("dm-7f3a", &m).unwrap()), "just us");
        // The voice key: alike for members, unknowable to others, new each epoch.
        let ka = a.export("dm-7f3a", "cave-den voice", 32).unwrap();
        assert_eq!(ka, b.export("dm-7f3a", "cave-den voice", 32).unwrap());
        assert!(c.export("dm-7f3a", "cave-den voice", 32).is_err());
        let kps_c = c.key_packages(1).unwrap();
        let (commit2, welcome2) = a.add("dm-7f3a", &kps_c).unwrap();
        a.confirm("dm-7f3a").unwrap();
        b.receive("dm-7f3a", &commit2).unwrap();
        c.accept(&welcome2).unwrap();
        let k2 = a.export("dm-7f3a", "cave-den voice", 32).unwrap();
        assert_ne!(k2, ka, "a new epoch, a new key");
        assert_eq!(k2, c.export("dm-7f3a", "cave-den voice", 32).unwrap());
        // A device already in the room cannot be added a second time.
        assert!(a.add("dm-7f3a", &kps_b[1..]).is_err(), "B's key is already a member");
    }

    /// The token round trip against a door key: what the door signs is the
    /// blinded message only, and the unblinded signature verifies on the token.
    #[test]
    fn a_blind_token_round_trip() {
        use blind_rsa_signatures::{DefaultRng, KeyPair, Randomized, Sha384, PSS};
        let kp = KeyPair::<Sha384, PSS, Randomized>::generate(&mut DefaultRng, 2048).unwrap();
        let der = kp.pk.to_der().unwrap();
        let p = token::blind(&der, "2026-10-12", "ab".repeat(32).as_str()).unwrap();
        assert!(!p.blinded.contains(&hex::encode("2026-10-12")), "the door does not see the day or the key");
        let blind_sig = kp.sk.blind_sign(hex::decode(&p.blinded).unwrap()).unwrap();
        let sig = token::finalize(&der, &p, &blind_sig.0).unwrap();
        // The relay verifies (randomizer ‖ msg) with RSA-PSS SHA-384 — what Elixir's :public_key does.
        let randomizer: [u8; 32] = hex::decode(&p.randomizer).unwrap().try_into().unwrap();
        let s = blind_rsa_signatures::Signature(hex::decode(&sig).unwrap());
        assert!(kp.pk.verify(&s, Some(blind_rsa_signatures::MessageRandomizer(randomizer)), p.msg.as_bytes()).is_ok());
    }
}
