//! A pager you can also ask questions.
//!
//! The alerts already exist and already work — `alert.rs` detected the halt on
//! 9 Sep, announced it, and announced the recovery. What it lacked was anywhere
//! to send them: `ZYN_ALERT_URL` was unset, so a stuck chain sat in journald
//! for a day and a half. Point that variable at Telegram's `sendMessage` and
//! the push half is solved with no code, because the webhook body already
//! carries a `text` field and Telegram takes `chat_id` from the query string.
//!
//! This binary is the other half: the ability to *ask*. Alerts tell you when
//! something broke; they cannot tell you the state of a thing that is merely
//! slow, or answer "is it back yet" without waiting for the next poll.
//!
//! It runs beside the sequencer and reads only what the RPC already exposes.
//! It holds no key, signs nothing, and submits nothing — every op it calls is
//! a public read, so the worst a compromised bot token can do is tell someone
//! the chain's height, which the DA store publishes anyway.

use std::time::Duration;


fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

struct Bot {
    token: String,
    /// Chats allowed to talk to it. Empty means answer nobody — a bot that
    /// answers anyone is a bot whose token is a disclosure.
    allowed: Vec<i64>,
    agent: ureq::Agent,
    /// Where the object ledger answers. It speaks JSON over HTTP; the AMM
    /// sequencer spoke a binary framing on a raw socket, and pointing this at
    /// the new chain was never a matter of changing a port.
    object: String,
    /// The chain it reports on, for the heading. Read from the node's own
    /// reply where possible — a bot that prints a chain id from its config
    /// will happily tell you about a chain it is not actually watching.
    chain: u32,
    /// `(label, host:port)` for each Zcash node worth watching.
    zebras: Vec<(String, String)>,
}

/// An HTTP error names the URL it failed on, and a Telegram URL carries the
/// bot's token. Written to journald as it was, every timed-out poll put the
/// token in the system log for anyone who can read it.
fn redact(message: &str, token: &str) -> String {
    if token.is_empty() {
        return message.to_string();
    }
    message.replace(token, "<token>")
}

impl Bot {
    fn api(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, method)
    }

    fn call(&self, method: &str, body: serde_json::Value) -> Option<serde_json::Value> {
        match self.agent.post(&self.api(method)).send_json(body) {
            Ok(r) => r.into_json::<serde_json::Value>().ok(),
            Err(e) => {
                eprintln!("zyn-tgbot: {} failed: {}", method, redact(&e.to_string(), &self.token));
                None
            }
        }
    }

    fn keyboard() -> serde_json::Value {
        serde_json::json!({
            "inline_keyboard": [
                [{"text": "chain", "callback_data": "/status"},
                 {"text": "signers", "callback_data": "/signers"}],
                [{"text": "anchors", "callback_data": "/anchors"},
                 {"text": "offers", "callback_data": "/offers"}],
                [{"text": "zcash nodes", "callback_data": "/nodes"}],
                [{"text": "whitelist", "callback_data": "/list"}],
            ]
        })
    }

    fn say(&self, chat: i64, text: &str) {
        self.call(
            "sendMessage",
            serde_json::json!({
                "chat_id": chat,
                "text": text,
                "parse_mode": "HTML",
                "disable_web_page_preview": true,
                "reply_markup": Self::keyboard(),
            }),
        );
    }

    /// Height, epoch, and — the number that mattered on 9 Sep — how far the
    /// last anchor is behind the tip. A sequencer waiting on signatures looks
    /// healthy in every other respect.
    /// Ask the object ledger. Every method here is a public read.
    fn object(&self, method: &str) -> Result<serde_json::Value, String> {
        self.object_at(&self.object, method)
    }

    fn object_at(&self, url: &str, method: &str) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method });
        let v: serde_json::Value = self
            .agent
            .post(url)
            .timeout(Duration::from_secs(15))
            .send_json(body)
            .map_err(|e| e.to_string())?
            .into_json()
            .map_err(|e| format!("unreadable reply: {}", e))?;
        if let Some(e) = v.get("error") {
            return Err(e.get("message").and_then(|m| m.as_str()).unwrap_or("error").to_string());
        }
        v.get("result").cloned().ok_or_else(|| "no result".to_string())
    }

    fn status(&self) -> String {
        match self.object("zyn_status") {
            Ok(s) => render_status(&s, self.chain),
            Err(e) => format!("chain {}: unreachable — {}", self.chain, e),
        }
    }
}

/// Turn a status document into the message a phone shows.
///
/// Separated from fetching it so it can be tested against a real reply: the
/// bot spent a day reporting confidently on a chain that had been retired,
/// and the way that goes unnoticed is that nothing ever checks the words.
fn render_status(s: &serde_json::Value, fallback_chain: u32) -> String {
    {
        let u = |k: &str| s.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        let t = |k: &str| s.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let chain = s.get("chain").and_then(|x| x.as_u64()).unwrap_or(fallback_chain as u64);

        let mut out = format!(
            "<b>chain {}</b>\nepoch <b>{}</b>  seq {}\n{} object(s) live  {} retired  {} account(s)",
            chain,
            u("epoch"),
            u("seq"),
            u("live"),
            u("nullifiers"),
            u("accounts"),
        );

        // The conservation check, which is the object ledger's equivalent of
        // "does the money add up": every object ever created is either still
        // held or retired exactly once. Anyone can verify it from this same
        // endpoint, which is why it is reported rather than asserted (A120).
        out.push_str(&if s.get("reconciled").and_then(|x| x.as_bool()) == Some(true) {
            format!("\n\u{2713} reconciled ({} commitments = {} live + {} spent)", u("commitments"), u("live"), u("nullifiers"))
        } else {
            format!("\n\u{26a0} NOT RECONCILED — {} commitments, {} live, {} spent", u("commitments"), u("live"), u("nullifiers"))
        });

        if u("offers") > 0 || u("exits") > 0 {
            out.push_str(&format!("\n{} resting offer(s)  {} exit(s)", u("offers"), u("exits")));
        }

        // Anchoring is the half that can fail silently: the ledger keeps
        // sequencing perfectly while nothing reaches Zcash, and only the lag
        // says so.
        let anchoring = t("anchoring");
        if anchoring.starts_with("none") {
            out.push_str(&format!("\n\u{26a0} anchoring {}", anchoring));
            return out;
        }
        let anchored = u("anchored_epoch");
        if anchored == 0 {
            out.push_str("\n\u{26a0} no anchor has landed yet");
        } else {
            let lag = u("epoch").saturating_sub(anchored);
            // One a day, so an epoch lag is not the unit that matters; what
            // matters is whether the last one landed and how far the ledger
            // has run since.
            let mark = if lag > 2000 { "\u{26a0}" } else { "\u{2713}" };
            out.push_str(&format!(
                "\n{} last anchor epoch <b>{}</b> at Zcash height {} (lag {} epochs)",
                mark, anchored, u("anchored_height"), lag
            ));
        }

        // Why no anchor has landed is nearly always this: the vault's note
        // trees are still walking to the tip, so there is no spendable note
        // to anchor from yet. Reporting the distance turns a mystery into a
        // wait with an end.
        let (synced, target) = (u("synced_to"), u("sync_target"));
        if target > 0 && synced < target {
            out.push_str(&format!(
                "\n\u{23f3} vault notes {} of {} blocks ({} behind)",
                synced,
                target,
                target - synced
            ));
        } else if target > 0 {
            out.push_str(&format!("\n\u{2713} vault notes at the tip ({})", synced));
        }
        let err = t("sync_error");
        if !err.is_empty() {
            out.push_str(&format!("\n\u{26a0} {}", err.chars().take(160).collect::<String>()));
        }
        out
    }
}

impl Bot {
    /// What the chain depends on, and whether each of those is working.
    ///
    /// The AMM sequencer polled several price sources and reported one health
    /// row per source. The object ledger has no price sources: what it depends
    /// on is the vault's view of Zcash, so that is what this reports. The
    /// command keeps its old name because that is what the muscle memory
    /// types at 3am.
    fn signers(&self) -> String {
        let s = match self.object("zyn_status") {
            Ok(s) => s,
            Err(e) => return format!("unreachable — {}", e),
        };
        let u = |k: &str| s.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        let anchoring = s.get("anchoring").and_then(|x| x.as_str()).unwrap_or("");
        let mut out = String::from("<b>what the chain rests on</b>\n");

        out.push_str(&if anchoring.starts_with("none") {
            format!("\u{26a0} <b>anchoring</b> — {}\n", anchoring)
        } else {
            format!("\u{2713} <b>anchoring</b> — {}\n", anchoring)
        });

        let (synced, target) = (u("synced_to"), u("sync_target"));
        if target == 0 {
            out.push_str("\u{26a0} <b>vault notes</b> — no Zcash tip read yet\n");
        } else if synced < target {
            out.push_str(&format!(
                "\u{23f3} <b>vault notes</b> — scanned to {} of {}, {} behind\n",
                synced,
                target,
                target - synced
            ));
        } else {
            out.push_str(&format!("\u{2713} <b>vault notes</b> — at the tip ({})\n", synced));
        }

        let err = s.get("sync_error").and_then(|x| x.as_str()).unwrap_or("");
        if !err.is_empty() {
            out.push_str(&format!("    {}\n", err.chars().take(160).collect::<String>()));
        }

        // The ledger's own integrity, which no external source can vouch for.
        out.push_str(&if s.get("reconciled").and_then(|x| x.as_bool()) == Some(true) {
            "\u{2713} <b>conservation</b> — every object held once or retired once\n".to_string()
        } else {
            "\u{26a0} <b>conservation</b> — the object counts do not balance\n".to_string()
        });
        out
    }

    /// The epoch-to-Zcash mapping, newest last. This is the thing worth being
    /// able to check from a phone: whether roots are still reaching Zcash.
    fn anchors(&self) -> String {
        let a = match self.object("zyn_anchors") {
            Ok(v) => v,
            Err(e) => return format!("anchors unavailable — {}", e),
        };
        let Some(rows) = a.as_array() else {
            return "anchors unavailable — unexpected reply".into();
        };
        if rows.is_empty() {
            // Not an error, and usually not a fault: say what to check next
            // rather than leaving a bare negative.
            return "no anchors published yet — /signers says what is holding it up".into();
        }
        let mut out = String::from("<b>recent anchors</b>\n");
        for x in rows {
            let g = |k: &str| x.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
            out.push_str(&format!(
                "epoch <b>{}</b> @ height {}\n  <code>{}</code>\n",
                g("epoch"),
                g("height"),
                x.get("txid").and_then(|v| v.as_str()).unwrap_or("?")
            ));
        }
        out
    }

    /// Both Zcash nodes: height against the network's own estimate of the tip.
    ///
    /// A node that is merely *behind* looks identical to one that is healthy if
    /// you only read its height, so the gap is the number reported. Zebra's
    /// `estimatedheight` is the network's view, so `lag` is honest even when
    /// the node is the thing that is wrong.
    fn nodes(&self) -> String {
        let mut out = String::from("<b>zcash nodes</b>\n");
        for (label, addr) in &self.zebras {
            let url = format!("http://{}/", addr);
            let body = serde_json::json!({
                "jsonrpc": "1.0", "id": 1, "method": "getblockchaininfo", "params": []
            });
            match self
                .agent
                .post(&url)
                .timeout(Duration::from_secs(8))
                .send_json(body)
            {
                Err(e) => {
                    let msg = e.to_string();
                    out.push_str(&format!(
                        "\u{26a0} <b>{}</b> unreachable\n    {}\n",
                        label,
                        msg.chars().take(70).collect::<String>()
                    ));
                }
                Ok(r) => {
                    let v: serde_json::Value = r.into_json().unwrap_or(serde_json::Value::Null);
                    let res = v.get("result").unwrap_or(&serde_json::Value::Null);
                    let blocks = res.get("blocks").and_then(|x| x.as_u64()).unwrap_or(0);
                    let tip = res
                        .get("estimatedheight")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0);
                    let chain = res.get("chain").and_then(|x| x.as_str()).unwrap_or("?");
                    let lag = tip.saturating_sub(blocks);
                    // Two blocks of slack: mainnet produces one every ~75s, so
                    // a node exactly at tip still reads one behind much of the time.
                    let mark = if lag <= 2 { "\u{2713}" } else { "\u{26a0}" };
                    out.push_str(&format!(
                        "{} <b>{}</b> ({})  height <b>{}</b>\n    tip {}  lag {}\n",
                        mark, label, chain, blocks, tip, lag
                    ));
                }
            }
        }
        out
    }

    /// Resting listings in the marketplace.
    ///
    /// The AMM had an order book of asset-for-asset offers. This is the Cave
    /// store's listings: one object, one price. Sellers are not shown — a
    /// listing is public, who placed it is not.
    fn offers(&self) -> String {
        let v = match self.object("zyn_offers") {
            Ok(v) => v,
            Err(e) => return format!("offers unavailable — {}", e),
        };
        let total = v.get("total").and_then(|x| x.as_u64()).unwrap_or(0);
        if total == 0 {
            return "no resting offers".into();
        }
        let empty = Vec::new();
        let rows = v.get("offers").and_then(|x| x.as_array()).unwrap_or(&empty);
        let mut out = format!("<b>{} resting offer(s)</b>\n", total);
        for x in rows {
            let item = x.get("item").and_then(|s| s.as_str()).unwrap_or("?");
            out.push_str(&format!(
                "<code>{}\u{2026}</code> \u{2192} {} of asset {}\n",
                &item[..item.len().min(16)],
                x.get("price").and_then(|s| s.as_str()).unwrap_or("?"),
                x.get("price_asset")
                    .and_then(|s| s.as_str())
                    .map(|s| s[..s.len().min(8)].to_string())
                    .unwrap_or_default(),
            ));
        }
        if total as usize > rows.len() {
            out.push_str(&format!("\n\u{2026} and {} more", total as usize - rows.len()));
        }
        out
    }

    /// The whitelist register.
    ///
    /// Read from the site rather than from a database of its own: the register
    /// lives where the form writes it, and a second copy would only be a second
    /// thing to keep true. The bot holds a read-only token for it — unrelated
    /// to anything that can spend or sign, so this command widens what a
    /// compromised bot discloses and not what it can do.
    fn whitelist(&self) -> String {
        let (Some(url), Some(token)) = (env("ZYN_WHITELIST_URL"), env("ZYN_WHITELIST_TOKEN"))
        else {
            return "whitelist is not wired up: set ZYN_WHITELIST_URL and ZYN_WHITELIST_TOKEN".into();
        };
        let res = self
            .agent
            .get(&url)
            .set("authorization", &format!("Bearer {}", token))
            .call();
        let v: serde_json::Value = match res {
            Ok(r) => match r.into_json() {
                Ok(v) => v,
                Err(e) => return format!("whitelist: unreadable reply: {}", e),
            },
            Err(e) => return format!("whitelist: {}", e),
        };
        let apps = v.get("applications").and_then(|a| a.as_array());
        let Some(apps) = apps else {
            return format!(
                "whitelist: {}",
                v.get("error").and_then(|e| e.as_str()).unwrap_or("no applications field")
            );
        };
        if apps.is_empty() {
            return "no applications yet".into();
        }
        let mut out = format!("{} application(s)\n", apps.len());
        // Newest last, so the most recent is nearest the reply — a phone shows
        // the bottom of a long message first.
        for a in apps.iter().rev().take(30).collect::<Vec<_>>().iter().rev() {
            let handle = a.get("handle").and_then(|h| h.as_str()).unwrap_or("?");
            let at = a.get("at").and_then(|t| t.as_str()).unwrap_or("");
            let post = a.get("post").and_then(|p| p.as_str()).unwrap_or("");
            out.push_str(&format!("\n@{}  {}\n{}\n", handle, &at[..at.len().min(10)], post));
        }
        if apps.len() > 30 {
            out.push_str(&format!("\n… and {} more, see the site", apps.len() - 30));
        }
        if v.get("truncated").and_then(|t| t.as_bool()) == Some(true) {
            out.push_str("\n(the listing was truncated)");
        }
        out
    }

    fn answer(&self, cmd: &str) -> String {
        match cmd.split_whitespace().next().unwrap_or("").trim_start_matches('/') {
            "status" | "start" => self.status(),
            "signers" | "health" => self.signers(),
            "anchors" => self.anchors(),
            "offers" => self.offers(),
            "nodes" | "zebra" => self.nodes(),
            "list" | "waitlist" | "whitelist" => self.whitelist(),
            "help" => "/status  epoch, objects, last anchor\n/signers  anchoring + vault note sync\n/list  whitelist applications\n/anchors  recent roots on Zcash\n/offers  resting marketplace listings\n/nodes  zcash mainnet + testnet sync".into(),
            other => format!("unknown command: {}\ntry /help", other),
        }
    }

    /// Look at the chain every few minutes and push what changed.
    /// Watch one chain. Run once per chain in `ZYN_WATCH`.
    fn watch(&self, url: &str) {
        let mut prev: Option<serde_json::Value> = None;
        let mut down = false;
        loop {
            match self.object_at(url, "zyn_status") {
                Ok(now) => {
                    if down {
                        self.tell(&format!("\u{2713} {url}: reachable again"));
                        down = false;
                    }
                    // The first look compares against a baseline that differs
                    // only in the standing faults, so a scan that was already
                    // stopped when the bot started is still reported — while
                    // anchors and deposits that already existed are not news.
                    let base = prev.clone().unwrap_or_else(|| baseline(&now));
                    for a in alerts(Some(&base), &now) {
                        self.tell(&a);
                    }
                    // Kept across an outage, so whatever changed while the node
                    // was unreachable is still compared against what came before.
                    prev = Some(now);
                }
                // Once per outage, not every five minutes: a pager that repeats
                // itself gets muted.
                Err(e) => {
                    if !down {
                        self.tell(&format!("\u{26a0} {url}: unreachable — {e}"));
                        down = true;
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(300));
        }
    }

    fn tell(&self, text: &str) {
        for chat in &self.allowed {
            self.say(*chat, text);
        }
    }

    fn run(&self) {
        let mut offset: i64 = 0;
        eprintln!(
            "zyn-tgbot: polling; answering {} chat(s); chain {} at {}",
            self.allowed.len(),
            self.chain,
            self.object
        );
        loop {
            let body = serde_json::json!({ "timeout": 50, "offset": offset });
            let Some(v) = self.call("getUpdates", body) else {
                std::thread::sleep(Duration::from_secs(5));
                continue;
            };
            let Some(items) = v.get("result").and_then(|r| r.as_array()) else {
                continue;
            };
            for u in items {
                if let Some(id) = u.get("update_id").and_then(|x| x.as_i64()) {
                    offset = id + 1;
                }
                // A button press and a typed command are the same question;
                // only where the text comes from differs.
                let (chat, text, callback) = if let Some(cb) = u.get("callback_query") {
                    (
                        cb.pointer("/message/chat/id").and_then(|x| x.as_i64()),
                        cb.get("data").and_then(|x| x.as_str()).map(str::to_string),
                        cb.get("id").and_then(|x| x.as_str()).map(str::to_string),
                    )
                } else {
                    (
                        u.pointer("/message/chat/id").and_then(|x| x.as_i64()),
                        u.pointer("/message/text")
                            .and_then(|x| x.as_str())
                            .map(str::to_string),
                        None,
                    )
                };
                if let Some(cb) = callback {
                    self.call(
                        "answerCallbackQuery",
                        serde_json::json!({ "callback_query_id": cb }),
                    );
                }
                let (Some(chat), Some(text)) = (chat, text) else {
                    continue;
                };
                if !self.allowed.contains(&chat) {
                    eprintln!("zyn-tgbot: ignoring chat {}", chat);
                    continue;
                }
                let reply = self.answer(&text);
                self.say(chat, &reply);
            }
        }
    }
}

/// What changed between two status documents that someone should be told.
///
/// The bot answered questions and pushed nothing, so the events that need a
/// person — the scan stopping on a payment it cannot attribute, the first
/// anchor landing, a deposit becoming an object — sat in a status document
/// until someone happened to type `/status`. A stopped scan in particular
/// waits on an operator: nothing moves until the transaction is attributed.
///
/// `prev` is `None` on the first look, which reports nothing: a restart of
/// the bot is not news.
/// `now` with its standing faults cleared: the comparison point for a first
/// look, so a fault already present is reported and nothing else is.
fn baseline(now: &serde_json::Value) -> serde_json::Value {
    let mut b = now.clone();
    if let Some(o) = b.as_object_mut() {
        o.insert("sync_error".into(), serde_json::json!(""));
        o.insert("reconciled".into(), serde_json::json!(true));
    }
    b
}

fn alerts(prev: Option<&serde_json::Value>, now: &serde_json::Value) -> Vec<String> {
    let Some(prev) = prev else { return Vec::new() };
    let u = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let t = |v: &serde_json::Value, k: &str| {
        v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    let chain = u(now, "chain");
    let mut out = Vec::new();

    let (was, is) = (t(prev, "sync_error"), t(now, "sync_error"));
    if !is.is_empty() && is != was {
        out.push(format!(
            "\u{26a0} <b>chain {chain}: scan stopped</b>\n{}",
            is.chars().take(300).collect::<String>()
        ));
    } else if is.is_empty() && !was.is_empty() {
        out.push(format!("\u{2713} chain {chain}: scan running again"));
    }

    if u(prev, "anchored_epoch") == 0 && u(now, "anchored_epoch") > 0 {
        out.push(format!(
            "\u{2713} <b>chain {chain}: first anchor on Zcash</b>\nepoch {} at height {}\n<code>{}</code>",
            u(now, "anchored_epoch"),
            u(now, "anchored_height"),
            t(now, "anchor_txid")
        ));
    }

    let (d0, d1) = (u(prev, "deposits_credited"), u(now, "deposits_credited"));
    if d1 > d0 {
        out.push(format!(
            "\u{2713} chain {chain}: {} deposit(s) credited ({d1} in all)",
            d1 - d0
        ));
    }

    let (o0, o1) = (u(prev, "owed_zatoshi"), u(now, "owed_zatoshi"));
    if o1 > o0 {
        out.push(format!(
            "chain {chain}: {} zat newly owed to exits ({o1} outstanding)",
            o1 - o0
        ));
    }

    let rec = |v: &serde_json::Value| v.get("reconciled").and_then(|x| x.as_bool());
    if rec(now) == Some(false) && rec(prev) != Some(false) {
        out.push(format!(
            "\u{26a0} <b>chain {chain}: NOT RECONCILED</b> — objects created no longer equal \
             objects held plus objects retired"
        ));
    }
    out
}

fn main() -> Result<(), String> {
    let token = env("ZYN_TG_TOKEN").ok_or("ZYN_TG_TOKEN is required")?;
    let allowed: Vec<i64> = env("ZYN_TG_CHATS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if allowed.is_empty() {
        return Err(
            "ZYN_TG_CHATS is required: a comma-separated list of chat ids allowed to ask".into(),
        );
    }
    // The object ledger speaks JSON over HTTP. `ZYN_NODE` named a raw socket
    // on the retired AMM chain; it is deliberately not reused, so a host that
    // still has the old variable set does not quietly look like it works.
    let object = env("ZYN_OBJECT_URL").unwrap_or_else(|| "http://127.0.0.1:8499/".into());
    // Two Zcash nodes, and neither is optional to the story: testnet is what
    // the object ledger anchors into, mainnet is what the vault custodies
    // against.
    let zebras: Vec<(String, String)> = vec![
        (
            "testnet".into(),
            env("ZYN_ZEBRA_TESTNET").unwrap_or_else(|| "127.0.0.1:18232".into()),
        ),
        (
            "mainnet".into(),
            env("ZYN_ZEBRA_MAINNET").unwrap_or_else(|| "127.0.0.1:8232".into()),
        ),
    ];
    let chain: u32 = env("ZYN_CHAIN_ID")
        .unwrap_or_else(|| "212".into())
        .parse()
        .map_err(|_| "ZYN_CHAIN_ID")?;

    let bot = Bot {
        token,
        allowed,
        agent: ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(60))
            .build(),
        object,
        chain,
        zebras,
    };
    let bot = std::sync::Arc::new(bot);
    // Every chain worth being paged about. Testnet's node answers on this box;
    // mainnet's is on Contabo, behind the public gateway's root since the AMM
    // chain that held 26460 was retired. Each alert names its chain, so one
    // chat can carry both without ambiguity.
    let watch: Vec<String> = env("ZYN_WATCH")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_else(|| vec![bot.object.clone()]);
    for url in watch {
        let bot = std::sync::Arc::clone(&bot);
        eprintln!("zyn-tgbot: watching {url}");
        std::thread::spawn(move || bot.watch(&url));
    }
    bot.run();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact document chain 212 returned on 22 Sep, so the rendering is
    /// tested against a real reply rather than one written to pass.
    const LIVE: &str = r#"{"chain":212,"epoch":1,"seq":2,"accounts":1,"commitments":1,
        "live":1,"nullifiers":0,"reconciled":true,"offers":0,"exits":0,
        "anchored_epoch":0,"anchored_height":0,"anchor_txid":"",
        "synced_to":4368400,"sync_target":4380781,"sync_error":"",
        "state_root":"d67c2fe014e71d950b246f00d6b8c1d6e7bbb64e3a5296915b0679b6441587e7",
        "anchoring":"on — roots are carried to Zcash and settle there"}"#;

    fn live() -> serde_json::Value {
        serde_json::from_str(LIVE).unwrap()
    }

    #[test]
    fn a_logged_error_never_carries_the_token() {
        let e = "https://api.telegram.org/bot123:ABC-def/getUpdates: timed out";
        let out = redact(e, "123:ABC-def");
        assert!(!out.contains("ABC-def"), "{out}");
        assert!(out.contains("getUpdates: timed out"));
    }

    /// The bug this whole change exists to fix: the bot reported chain 11
    /// long after chain 11 was retired, because the chain it named came from
    /// its own config rather than from the node answering.
    #[test]
    fn it_names_the_chain_the_node_reports_not_the_one_configured() {
        let out = render_status(&live(), 11);
        assert!(out.contains("chain 212"), "{out}");
        assert!(!out.contains("chain 11"), "it must not report a retired chain: {out}");
    }

    /// A chain that is anchoring but has not landed one yet must say both
    /// halves. Saying only "anchoring on" reads as success; saying only "no
    /// anchor" reads as broken. It is neither — it is waiting on the scan.
    #[test]
    fn a_chain_waiting_on_its_note_trees_says_so_and_says_how_far() {
        let out = render_status(&live(), 212);
        assert!(out.contains("no anchor has landed yet"), "{out}");
        assert!(out.contains("12381 behind"), "it must report the distance left: {out}");
    }

    /// Conservation is the object ledger's "does the money add up". A failure
    /// has to be unmissable, not a number the reader must compare themselves.
    #[test]
    fn a_ledger_that_does_not_balance_is_reported_as_a_warning() {
        let mut v = live();
        v["reconciled"] = serde_json::json!(false);
        v["live"] = serde_json::json!(5);
        let out = render_status(&v, 212);
        assert!(out.contains("NOT RECONCILED"), "{out}");

        assert!(render_status(&live(), 212).contains("reconciled"), "and says so when it does");
    }

    /// Once an anchor lands, the mapping to Zcash is the headline, and the
    /// note-tree line becomes a tick rather than a wait.
    #[test]
    fn a_landed_anchor_is_reported_with_its_height() {
        let mut v = live();
        v["anchored_epoch"] = serde_json::json!(4);
        v["anchored_height"] = serde_json::json!(4_380_900u64);
        v["epoch"] = serde_json::json!(5);
        v["synced_to"] = v["sync_target"].clone();
        let out = render_status(&v, 212);
        assert!(out.contains("last anchor epoch <b>4</b>"), "{out}");
        assert!(out.contains("4380900"), "{out}");
        assert!(out.contains("at the tip"), "{out}");
        assert!(!out.contains("no anchor has landed"), "{out}");
    }

    fn with(v: &serde_json::Value, k: &str, x: serde_json::Value) -> serde_json::Value {
        let mut v = v.clone();
        v[k] = x;
        v
    }

    /// The event that waits on a person: a payment with no memo stops the
    /// scan until someone attributes it. It must be pushed, with the txid.
    #[test]
    fn a_stopped_scan_is_pushed_with_its_reason() {
        let before = live();
        let after = with(&before, "sync_error", serde_json::json!(
            "stopped at 4380950: abc123 carries no Zyn memo — add it to ZYN_ATTRIBUTIONS"));
        let a = alerts(Some(&before), &after);
        assert_eq!(a.len(), 1, "{a:?}");
        assert!(a[0].contains("scan stopped") && a[0].contains("abc123"), "{}", a[0]);
        // And it is not repeated every five minutes while it stays stopped.
        assert!(alerts(Some(&after), &after).is_empty());
        // Clearing it is news too.
        assert!(alerts(Some(&after), &before)[0].contains("running again"));
    }

    /// A scan already stopped when the bot starts is a standing fault, not a
    /// restart artifact — the first look must still report it.
    #[test]
    fn a_stall_present_at_startup_is_still_reported() {
        let now = with(&live(), "sync_error", serde_json::json!("stopped at 1: x"));
        let a = alerts(Some(&baseline(&now)), &now);
        assert_eq!(a.len(), 1, "{a:?}");
        // But nothing that merely already existed is announced.
        let quiet = alerts(Some(&baseline(&live())), &live());
        assert!(quiet.is_empty(), "{quiet:?}");
    }

    #[test]
    fn the_first_anchor_and_new_deposits_are_pushed() {
        let before = with(&live(), "deposits_credited", serde_json::json!(1));
        let after = with(
            &with(&with(&before, "anchored_epoch", serde_json::json!(3)),
                  "anchored_height", serde_json::json!(4_381_500u64)),
            "deposits_credited", serde_json::json!(3),
        );
        let a = alerts(Some(&before), &after).join("\n");
        assert!(a.contains("first anchor") && a.contains("4381500"), "{a}");
        assert!(a.contains("2 deposit(s) credited"), "{a}");
        // A later anchor is not "first".
        let later = with(&after, "anchored_epoch", serde_json::json!(4));
        assert!(!alerts(Some(&after), &later).join("").contains("first anchor"));
    }

    #[test]
    fn losing_reconciliation_is_pushed_once() {
        let bad = with(&live(), "reconciled", serde_json::json!(false));
        assert!(alerts(Some(&live()), &bad)[0].contains("NOT RECONCILED"));
        assert!(alerts(Some(&bad), &bad).is_empty());
    }

    /// A node that is not anchoring at all must not be dressed up: the devnet
    /// status said "anchoring" while carrying nothing, and that is the wrong
    /// lie in either direction.
    #[test]
    fn a_chain_that_anchors_nowhere_is_reported_as_a_warning() {
        let mut v = live();
        v["anchoring"] = serde_json::json!("none — devnet, roots reach no chain");
        let out = render_status(&v, 212);
        assert!(out.contains("\u{26a0} anchoring none"), "{out}");
    }

    /// The scanner stall is the actionable case: it names a transaction the
    /// operator has to attribute by hand, so it must survive into the message.
    #[test]
    fn a_stalled_scan_reports_why() {
        let mut v = live();
        v["sync_error"] = serde_json::json!("stopped at 4368401: abc123 carries no Zyn memo");
        let out = render_status(&v, 212);
        assert!(out.contains("carries no Zyn memo"), "{out}");
    }
}
