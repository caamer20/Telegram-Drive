# Raw SendMedia Blocker Report

## Verdict

**TELEGRAM_RAW_SEND_BLOCKED** at pinned grammers revision `d07f96f`.

## Pinned Revision

- grammers commit: `d07f96f`
- Repository: `https://github.com/Lonami/grammers`

## Source Files

| Component | File |
|-----------|------|
| `Uploaded` struct | `grammers-client/src/types/media.rs:33` |
| `Uploaded` visibility | `grammers-client/src/types/mod.rs: pub(crate) use media::Uploaded;` |
| `send_media` → `send_message` | `grammers-client/src/client/messages.rs:490` |
| `generate_random_id` | `grammers-client/src/client/messages.rs` (internal) |
| `Client::invoke` | `grammers-client/src/client/client.rs` (public) |
| `SendMedia` (TL) | `grammers-tl-types` generated at `tl::functions::messages::SendMedia` |
| `InputMediaUploadedDocument` (TL) | `grammers-tl-types` generated at `tl::types::InputMediaUploadedDocument` |

## Blocker: `Uploaded` is `pub(crate)`

The `Uploaded` struct holds the raw TL `InputFile`:
```rust
#[derive(Clone, Debug, PartialEq)]
pub struct Uploaded {
    pub raw: tl::enums::InputFile,
}
```

But this struct is re-exported as `pub(crate)`:
```rust
// grammers-client/src/types/mod.rs
pub(crate) use media::Uploaded;
```

This means:
- We **can** call `client.upload_stream()` and get an `Uploaded` value
- We **cannot** import the `Uploaded` type
- We **cannot** access `.raw` to build `InputMediaUploadedDocument`

## Why `Client::invoke` Alone Is Not Enough

`Client::invoke(&request)` IS public and CAN be called. The problem is building the `request`:

```rust
tl::functions::messages::SendMedia {
    media: tl::enums::InputMedia::UploadedDocument(
        tl::types::InputMediaUploadedDocument {
            file: uploaded.raw.clone(),  // ← CANNOT ACCESS (pub(crate))
            ...
        }
    ),
    random_id: persisted_random_id,  // ← We want to inject this
    ...
}
```

We cannot construct `InputMediaUploadedDocument.file` without `uploaded.raw`.

## Why `send_message` Cannot Be Used

`client.send_message()` auto-generates `random_id`:
```rust
// grammers-client/src/client/messages.rs:500
let random_id = generate_random_id();
self.invoke(&tl::functions::messages::SendMedia {
    random_id,  // ← AUTO-GENERATED, not injectable
    ...
})
```

There is no public method that accepts a `random_id` parameter.

## Compiler Proof

```bash
$ cargo check --lib
error[E0432]: unresolved import `grammers_client::Uploaded`
  --> src/migration/adapters_v2/telegram.rs:18:25
   |
18 | use grammers_client::Uploaded;
   |                         ^^^^^^^^ no `Uploaded` in the root
```

And if we try to access the `.raw` field:
```rust
let uploaded = client.upload_stream(...).await?;
let raw_file = uploaded.raw; // ERROR: field `raw` of struct `Uploaded` is private
```

## Minimal Upstream Change Required

Make `Uploaded` public:

```diff
- pub(crate) use media::Uploaded;
+ pub use media::Uploaded;
```

Or add a method to inject `random_id`:

```rust
// In grammers-client
pub async fn send_media_with_id<C: Into<PeerRef>, M: Into<types::InputMessage>>(
    &self,
    peer: C,
    message: M,
    random_id: i64,
) -> Result<Message, InvocationError>
```

## Alternative: Fork/Patch

If forking grammers:
- Change `pub(crate) use media::Uploaded` → `pub use media::Uploaded`
- Risk: upstream breaking changes, maintenance burden
- Benefit: full idempotent SendMedia with persisted random_id

## Risk

- Without persisted `random_id`, retry after crash WILL send duplicate messages
- Reconciliation must be done via Telegram history (chat log scan) instead of deterministic ID
- Status quo: `client.send_message()` auto-gen IDs make every send a potential duplicate on retry

## Decision

**NO_GO**: Raw `SendMedia` with persisted `random_id` is blocked at pinned grammers revision.
The Telegram production adapter V2 will be kept compile-safe with typed errors, but the actual send operation will remain delegated to the high-level `client.send_message()` API with a clear warning.

This status will be revisited when:
1. Grammers bumps to a revision with public `Uploaded`
2. A fork makes `Uploaded` public
3. An alternative upload→send pipeline is discovered in the grammers API
