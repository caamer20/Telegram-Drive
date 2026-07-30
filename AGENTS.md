<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **Telegram-Drive** (5042 symbols, 9000 relationships, 300 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> If any GitNexus tool warns the index is stale, run `npx gitnexus analyze` in terminal first.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.

## Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/Telegram-Drive/context` | Codebase overview, check index freshness |
| `gitnexus://repo/Telegram-Drive/clusters` | All functional areas |
| `gitnexus://repo/Telegram-Drive/processes` | All execution flows |
| `gitnexus://repo/Telegram-Drive/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->

## Ứng dụng Android độc lập

- Project nằm tại `android-app/`; không sửa `app/src-tauri/gen/android` vì đó là output generated của Tauri.
- Dùng Kotlin, Compose, Kotlin DSL, minSdk 26 và application ID `com.nmtuong.telegramdrive`.
- Dependency hướng vào trong: UI/feature → repository → gateway; UI/domain không import `org.drinkless.tdlib`.
- TDLib chỉ lấy từ source Telegram chính thức và build bằng `scripts/build-tdlib-android.sh`; không thêm binary bên thứ ba.
- Real/fake source chọn bằng Gradle property `-PtelegramDataSource=real|fake`; không đưa credential/session thật vào source hoặc artifact.
- Trước handoff phải chạy `./gradlew testDebugUnitTest lintDebug assembleDebug` trong `android-app/` và dùng Android CLI/adb để install, launch, layout, screenshot khi có runtime.
- Giai đoạn 0 không gồm login thật, browsing, download/preview, Room, background transfer, release, CI/CD, MCP hoặc Lightbuild.
