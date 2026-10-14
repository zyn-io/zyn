// What an agent reads: one message, one line. The sender's kind comes from
// its key, never from anything it wrote, and its words are kept to one line,
// so no message can pass for a line of its own or for its owner's.

/// Who sent it, as this agent sees them: its owner (or a device linked to
/// the owner) carries orders; anyone else is information.
export const KINDS = ['owner', 'agent', 'member', 'self', 'den'];

/// Words on one line: line breaks shown as ⏎, other control and direction
/// characters taken out.
export function oneLine(s) {
  return String(s ?? '')
    .replace(/\r\n|\r|\n|\u2028|\u2029/g, ' ⏎ ')
    .replace(/[\u0000-\u001f\u007f-\u009f\u200B-\u200F\u202A-\u202E\u2060-\u2069\uFEFF\u061C]/g, '')
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
  };
}

/// A record as one plain line:
///   #general 42 14:02:11 owner m-1a2b… "Tobias" ↩41: words
export function line(r, now = Date.now()) {
  const who = r.kind === 'den' ? 'den' : `${r.kind} ${r.from}${r.nick ? ` ${quoted(r.nick)}` : ''}`;
  const reply = r.reply ? ` ↩${Number(r.reply) || 0}` : '';
  const words = r.deleted ? '(deleted)' : `${oneLine(r.text)}${r.edited ? ' (edited)' : ''}`;
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
