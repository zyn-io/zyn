# Epeaky and A2A

[A2A](https://a2a-protocol.org) (Agent2Agent, 1.0) is how agents of
different makers hand each other work over HTTP: an **Agent Card** says what
an agent does and how to authenticate to it; a client sends a **Message**
(parts: text, files, data); the work is a **Task** with an id, a `contextId`
for the conversation, a history, artifacts and a state
(`submitted`, `working`, `input-required`, `auth-required`, `completed`,
`failed`, `canceled`, `rejected`). Identity is whatever the server's security
scheme says: an API key, OAuth, mutual TLS.

This note is what A2A means for Epeaky, and what to build.

## Where it does not fit

- **On the den.** An A2A server reads the words it is sent. The den sees
  only ciphertext; an A2A endpoint on the den would be a second, readable way
  in. There is none, and there will not be.
- **As identity.** In a den, who said something is the MLS key that signed
  it, and only the owner's words are orders (`authority: owner`). A2A's
  identity is a bearer credential at an HTTP endpoint. Nothing that arrives
  over A2A becomes an order by itself.
- **As discovery.** An Agent Card per Epeaky agent would publish that the
  agent exists and who runs it. Agents in a den are found by its members, in
  the den.

## Where it fits

### 1. Tasks speak A2A's words (now, no cost)

The task format in `epeaky-agent` uses A2A's states and shape, so a bridge
later is a mapping, not a translation:

| Epeaky | A2A |
|---|---|
| the order message (owner's words) | the `Message` that starts a `Task` (`role: user`) |
| task id: the order's message id | `Task.id` |
| the room (or the thread under the order) | `contextId` |
| state: `submitted`, `working`, `input-required`, `completed`, `failed`, `canceled`, `rejected` | `TaskState` of the same name |
| the agent's replies under the order | `history` (`role: agent`) |
| files the agent posts for the task | `artifacts` |

`auth-required` has no Epeaky meaning (the agent is already in) and is left
out.

### 2. `epeaky-a2a`: calling agents outside the den (next)

A process next to the agent, like `epeaky-mcp`, built on the command line.
An owner says, in the den, which outside agent to use; the bridge sends the
task there (`SendMessage`, or `SendStreamingMessage` and `SubscribeToTask`)
and posts its states and artifacts back under the order.

- **Owner only.** Only an order with `authority: owner` can send anything
  out; the outside agent's answers are posted by the agent, so the den reads
  them as an agent's words (`kind: agent`, `authority: none`), never orders.
- **Said where it happens.** The words leave the den's encryption for that
  agent's operator. The bridge says so in the room, with the endpoint,
  before it sends, and the list of endpoints it may use is the owner's, in a
  local file, never taken from a message.
- **Its credentials stay on its machine,** in the agent's folder, readable
  only by its user.

### 3. Taking tasks from outside (later, if asked for)

An A2A endpoint on the agent's machine that turns a caller's task into a
message in the den. Every such message is information unless its owner has
named that caller as one whose tasks it takes; that list, the endpoint's
exposure and its authentication are the hard part. Not built until someone
needs it.

## Order

1. Tasks with A2A's states and shape (gap 1, in `epeaky-agent`).
2. `epeaky-a2a`, outbound, once tasks exist; an e2e with a stub A2A agent.
3. Inbound only when a user asks for it.
