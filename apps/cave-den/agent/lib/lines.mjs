// What an agent reads: one message, one line. The sender's kind comes from
// its key, never from anything it wrote, and its words are kept to one line,
// so no message can pass for a line of its own or for its owner's.

/// Who sent it, as this agent sees them: its owner (or a device linked to
/// the owner) carries orders; anyone else is information.
export const KINDS = ['owner', 'agent', 'member', 'self', 'den'];

/// Words on one line: line breaks shown as ⏎ (and a ⏎ someone typed as
/// [⏎]), control, format, tag and direction characters and variation
/// selectors taken out, so nothing in them is hidden from a reader.
export function oneLine(s) {
  return String(s ?? '')
    .replace(/\u23CE/g, '[⏎]')
    .replace(/\r\n|\r|\n|\u2028|\u2029/g, ' ⏎ ')
    .replace(/[\p{Cc}\p{Cf}\u{E0000}-\u{E007F}\uFE00-\uFE0F\u{E0100}-\u{E01EF}]/gu, '')
    .replace(/ {2,}/g, ' ')
    .trim();
}

/// A nickname as a line shows it: in double quotes, so none of its own.
const quoted = (nick) => `"${oneLine(nick).replace(/"/g, "'")}"`;

function when(at, now = Date.now()) {
  const d = new Date(at);
  const iso = d.toISOString();
  return iso.slice(0, 10) === new Date(now).toISOString().slice(0, 10) ? iso.slice(11, 19) : `${iso.slice(0, 10)} ${iso.slice(11, 16)}`;
}

/// A log entry as a record: room, seq, kind, sender, words, and whether it
/// carries the owner's authority.
export function record(room, m, { kind, nick, name, mention = false, replySeq = null }) {
  const words = m.deleted ? '' : m.t === 'file' ? `[file] ${m.file?.name || 'file'}${m.text ? ` ${m.text}` : ''}` : m.text || '';
  return {
    room,
    seq: m.seq || 0,
    id: m.id,
    at: new Date(m.at || 0).toISOString(),
    kind,
    authority: kind === 'owner' ? 'owner' : 'none',
    from: name,
    key: m.key || m.who || null,
    nick: nick || '',
    text: words,
    reply: replySeq,
    reply_id: m.reply || null,
    mention: !!mention,
    ...(m.edited ? { edited: true } : {}),
    ...(m.deleted ? { deleted: true } : {}),
    ...(m.task ? { task: { id: m.id, to: m.task.to, state: m.task.state, text: m.task.text || '', text_by: m.task.by || null, artifacts: m.task.artifacts || [], updated: new Date(m.task.updated || m.at || 0).toISOString(), by: m.task.by || null, history: (m.task.history || []).map((h) => ({ state: h.state, by: h.by || null, at: new Date(h.at || 0).toISOString() })) } } : {}),
  };
}

/// A record as one plain line:
///   #general 42 14:02:11 owner m-1a2b… "Tobias" ↩41: words
export function line(r, now = Date.now()) {
  const who = r.kind === 'den' ? 'den' : `${r.kind} ${r.from}${r.nick ? ` ${quoted(r.nick)}` : ''}`;
  const reply = `${r.reply ? ` ↩${Number(r.reply) || 0}` : ''}${r.for ? ` for ${Number(r.for.seq) || 0}` : ''}`;
  // The state's words are whoever set the state's: named when that is not
  // the task's giver, so a worker's words never read as an order.
  const by = r.task?.text_by && r.task.text_by !== r.key ? `, from m-${String(r.task.text_by).slice(0, 20)}` : '';
  const task = r.task ? `[task ${oneLine(r.task.id)} for ${r.task.to_nick ? quoted(r.task.to_nick) : `m-${String(r.task.to).slice(0, 20)}`} · ${oneLine(r.task.state)}${r.task.text ? `${by}: ${oneLine(r.task.text)}` : ''}${r.task.artifacts?.length ? ` (results: ${r.task.artifacts.map(oneLine).join(', ')})` : ''}] ` : '';
  const change = r.change === 'edited' ? ' (edited)' : r.change === 'state' ? ' (task changed)' : '';
  const words = r.deleted ? `${task}(deleted)` : `${task}${oneLine(r.text)}${r.edited && r.change !== 'edited' ? ' (edited)' : ''}${change}`;
  return `#${oneLine(r.room).replace(/\s/g, '')} ${Number(r.seq) || 0} ${when(r.at, now)} ${who}${reply}: ${words}`;
}

/// Whether words name this agent: by its device name (as the page writes a
/// mention) or as @nickname.
export function names(text, { name, nick }) {
  const t = String(text || '');
  if (name && t.includes(`@${name}`)) return true;
  if (!nick) return false;
  const esc = nick.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`(^|[^\\p{L}\\p{N}_])@${esc}(?![\\p{L}\\p{N}_])`, 'iu').test(t);
}
