# Protocol

`crates/protocol` holds every message between `shipdeck engine` and its
apps. It has no engine logic: no SSH, no files, no build.

## The wire

- JSON-RPC 2.0, one message per line of UTF-8, over stdin and stdout.
- The app sends requests. The engine sends events (no id) and its own
  questions (with an id).
- Field names are `snake_case`. A value with variants is one flat object
  with a `kind` field, such as `{"kind": "key", "path": "..."}`.
- Names are a contract: add new methods and fields, never rename or remove
  one. A field marked `#[serde(default)]` is the optional one.
- Secrets travel in the pipe. A password is never written to a log.

## Adding a message

1. Add the params or result type in `src/message/request.rs`, `event.rs` or
   `ask.rs`, with `Serialize`, `Deserialize` and `JsonSchema`.
2. Add its name in `src/message/method.rs`.
3. Read it in `Event::from_wire` or `Ask::from_wire` when the engine sends it.
4. Add it to `messages()` in `src/schema.rs`.
5. Run `just protocol`; a test fails until `protocol.schema.json` is current.
6. Commit `protocol.schema.json` with the change. The desktop app rebuilds its
   C# types from it.

## Errors

`wire::code` lists every code once. JSON-RPC's own codes are `-327xx`;
shipdeck's start at `-32001` and go down. A code is never renumbered.

## The two ends

`connection::Connection::start(read, write)` takes any async reader and
writer, so a test can use an in-memory pipe and the app can use a child
process's pipes. `request` waits for the answer, `recv` yields events and
questions, and `answer` replies to a question. Dropping it closes the
stream.

Both sides use the same type. `handle()` gives the cloneable sending half,
so the tasks a worker starts can report while its own loop keeps reading:
the engine's deploy sends `op/*` events and `ask/*` questions from tasks.
`answer_error` answers with one of `wire::code`'s numbers.

## The engine process

`shipdeck engine` is `crates/engine/src/rpc`: it holds one connection's state,
the open workspace, the running deploy, and the live logs. It writes nothing
to stdout but messages.

- One request, one task. Answers come back as they finish, so the app
  matches ids. `initialize` and `workspace/open` are answered before the next
  message is read, so everything after them sees what they set.
- One deploy or rollback at a time. A second one is refused with `BUSY`.
- `logs/follow` answers before its first line, and `deploy/start` before its
  first `op/*` event: the app needs the view or the operation first.
- `shutdown` answers, then closes the SSH connections and exits. A request
  already running is still answered.
