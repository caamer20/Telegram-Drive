# Browser end-to-end tests

Run `npm run test:e2e` (or `npm test`) from `app/`. Install Chromium first with
`npx playwright install chromium`; CI uses `--with-deps`. Failed runs retain a
screenshot and Playwright trace. CI also writes `playwright-report/`.
CI keeps diagnostic retries, but a test that only passes on retry still fails the run.

`e2e/` drives the real app entry point and provider tree through authentication,
account retry and stale-result rejection, folder loading, sign-out recovery,
settings persistence, image preview, lifetime supporter recovery/ad suppression,
and updater retry/package-manager behavior. Native commands and external services
are replaced at the Tauri boundary with deterministic browser fixtures. Assertions
check visible results after user actions, with boundary call inspection for
forbidden actions such as repeat checkout, early install, or premature logout
cleanup. No real payment, Telegram account, credential store or application
installer is contacted.

`visual/` retains the existing browser journeys for Android layout/navigation and
transfers, workspace collections and media galleries, storage cleanup, collision
choices, confirmation dialogs, supporter presentation, accessibility and snapshots.
Some of these mount a development-only feature fixture rather than the whole app;
they supplement the complete app journeys and are not native-device E2E proof.

These browser runs do not establish native SQLite/locking behavior, cryptographic
correctness, real Telegram transfer reliability, real OS updater installation,
Android lifecycle/Keystore behavior, or PayPal processing. Those require the native
and service E2E suites plus isolated device and payment-sandbox release exercises.
Do not replace missing native coverage with another browser stub or claim that a
passing browser suite proves the entire system defect-free.

New regression coverage should exercise an observable user journey. Keep build,
type, localization, configuration-policy and dependency checks as static validation;
do not introduce another unit runner or isolated function assertion suite.

## Consolidated browser journeys

| Former standalone coverage | Owning journey | Checks to retain |
| --- | --- | --- |
| All presets and Default Dark border (two registrations) | `settings-themes.spec.ts` base modes and presets | All 15 stable IDs, selected state, label contrast, Default Dark border and Sky after reload. Exercise manual Performance Mode on and off, plus the OS reduced-motion setting. |
| OS preference changes with a saved-off manual setting (one registration) | `motion-policy.spec.ts` saved Performance Mode | Wait two animation frames and verify the saved-off switch, persistence after restart and the separately exercised manually toggled-off state. |
| Rapid settings categories and immediate dismissal (one registration) | `settings-themes.spec.ts` rapid categories and nested explanation | Check exiting panes, input release on Done, reopening, Escape, nested explanation dismissal and return focus. |
