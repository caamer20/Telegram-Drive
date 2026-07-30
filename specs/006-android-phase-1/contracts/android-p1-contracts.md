# Contracts: Android Phase 1

## Repository contract

- Expose observable authorization, library và download states thuần domain.
- Accept auth action chỉ khi action hợp current state và không pending; trả rejected result cho invalid/duplicate.
- `loadSavedMessages(limit)` chỉ hợp lệ khi Ready; limit tối đa 50.
- `download(fileId)` và `cancel(fileId)` idempotent; không block caller.
- `logout/reset` là explicit lifecycle owner và cleanup account state.

## Gateway contract

- Một native client và một receive loop.
- Mọi request có unique `@extra`; response hoàn tất đúng pending request hoặc bị bỏ qua an toàn nếu stale.
- Update authorization/file được map và phát theo thứ tự nhận.
- Không log raw auth request hoặc input secret; error qua redaction.

## UI contract

- Screen auth chỉ render action hợp state; repeated click khi pending không gửi thêm request.
- Library có loading/content/empty/error; stable item key.
- Preview route chỉ nhận verified local path; image/video có back/error.
- Video player không tồn tại ngoài preview owner và release đúng một lần.

## Fake contract

- Cùng domain states/actions như real source.
- Flow deterministic gồm auth states, Ready, Saved items, duplicate/out-of-scope content, download progress/success/failure/cancel, image/video local fixtures.
- Không tham chiếu hoặc load `JsonClient`/native library.
