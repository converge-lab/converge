# UI error visibility

Audit and implementation: 2026-09-20. The reported symptom was a failed group
invitation with no visible explanation. Its production response was not
captured, so the exact cause of that incident remains unverified.

## Agreed behavior

An operation shows progress, prevents duplicate submissions, and explains its
outcome. Failed forms retain their inputs. Create/delete dialogs close only
when the server confirms success. Errors stay beside their action; a failure
arriving after that view disappears becomes a persistent shell notification.
Independent notifications do not replace each other. Only success receipts
expire automatically.

Error handling considers two audiences separately: users receive understandable
domain explanations, while diagnostics must preserve useful technical
causes without confidential information. Internal failures receive a general
user-facing message. This policy is recorded in Converge as
`01M2Z18KZDMRZSEDX4KGPM3KXP`.

**Browser diagnostic logging is deferred.** There is no approved collection
backend yet. This change adds no Sentry integration, console logging, remote
telemetry, or local diagnostic buffer. `feedback::message` formats user-facing
text without side effects; `ActionState` manages pending/error presentation.
The deferred collection policy is recorded separately as
`01M2ZV6KKVCRWQAJYRHBKPPA6K`; it preserves the earlier privacy requirements.

## Covered surfaces

| Surface | Result |
| --- | --- |
| Add teammate | Keep the modal and handle on failure; show an inline alert and progress; block duplicate submits. After dismissal/navigation, use a shell notification. |
| Handle lookup | Preserve provider casing; trim whitespace and the display `@`. Storage matches handles exactly. |
| Member roster/removal | Distinguish loading from failure and allow Retry. Removal shows progress and preserves the row on failure. Late failures remain visible. |
| Group/project creation and deletion | Keep the dialog and entered values on refusal. Navigate and update the dataset after confirmed success. Group deletion reads the captured store so asynchronous completion does not panic outside component context. |
| Group/project settings | Show progress and errors beside Save; preserve name/description on failure. |
| Signal verdicts | Keep the screen on refusal; disable repeated submissions and show the outcome. |
| Search | Distinguish loading, failure, and successful empty results; offer Retry. Discard responses to superseded searches. |
| Decision sources/relations | Show loading and an inline error with Retry. Retain the previous dataset if either request fails. |
| API tokens | Distinguish failed loading from an empty list. Preserve creation input and revocation confirmation on failure. A token minted after navigation produces a recovery message without exposing its secret. |
| Device pairing | Show pending and retryable failures for lookup and approval/denial. Preserve the code and prevent concurrent verdicts. |
| Authentication | Show provider discovery and sign-in failures separately; allow provider Retry. Explain sign-out failures and offer Retry after boot failure. |
| Shell notifications | Queue independent outcomes. Dismissing or timing out one receipt cannot erase another error. |

## Review follow-up

The follow-up addresses roster action lifetime, deletion/navigation races,
notification behavior, disabled inputs, safe database errors, and executable
browser checks. Search changes remain deferred at the user's request.

- Member rows are keyed by user ID. Their pending state and error survive
  removal of another member and invitation-triggered roster refreshes; updated
  display names are still reflected. The roster load alert stays mounted.
- A completed deletion leaves a removed object's page even when its dialog
  was dismissed while waiting. Navigation elsewhere is preserved, as is a
  different selected group. Group removal publishes one coherent dataset
  update; the router redirects missing project/group settings targets before
  rendering editable controls or loading an invalid roster.
- Disabled inputs have distinct text/background colors and a disabled cursor.
- Repeated failures with the same operation/resource identity and message use
  one receipt with an occurrence count. Equal text from different operations
  does not merge. The latest five receipts are shown; all other unread errors
  remain available through "Show all". Repeats return to the visible list.
  Gaps pass pointer events through, and an explicitly expanded history is a
  visible scrolling panel. Notifications stay below the modal interaction
  layer so accumulated receipts cannot block another form. Success timers
  belong to receipts and run independently of whether they are folded.
- PostgreSQL error mapping never copies driver text into `Invalid`/`Conflict`.
  Its server diagnostics contain a fixed category and validated SQLSTATE,
  without raw messages, SQL, parameters, identifiers, or connection URLs.
  The HTTP error boundary also avoids logging raw backend causes from other
  implementations. This adds no browser diagnostic logging.
- CI builds the API WASM bundle through Trunk/wasm-bindgen and runs Playwright,
  covering failures that `cargo check` alone cannot detect.

The membership endpoint adds an **existing** user by handle. It does not send
email invitations. A user must sign in once before they can be added.

The API browser build exposed a dependency conflict: reqwest 0.12 streaming
uses `wasm-streams` 0.4, while the current Leptos `server_fn` uses 0.5, producing
duplicate WASM exports. The browser client uses reqwest 0.13 to align them;
native clients retain reqwest 0.12.

## Explicitly deferred

- The expert chat is excluded at the user's request. Its chat list, composer,
  streaming behavior, and error handling are unchanged. The intended separate
  redesign has one chat window without a chat list.
- Sentry, other browser diagnostic destinations, and request-id correlation.
- Search refresh/debounce and validation-specific Retry behavior. The existing
  search implementation and its baseline regression checks are unchanged.
- The rare successful-sign-in/failed-navigation presentation and remaining
  success-notification differences in the embedded build.
- Preserving drafts through a complete sign-in/reload flow and auditing
  permissions of all displayed actions.

## Verification

- `cargo test -p converge-web --all-features --lib`: 21 passing tests cover safe error text,
  case-preserving handles, independent notification lifetimes, and failures
  after the form's reactive owner is disposed. A separate regression covers
  group deletion after an API response, without component context, preserving
  another selected group, and notification deduplication by resource identity.
- `cargo test -p converge-storage-postgres --lib error::tests`: two passing
  tests verify sanitized diagnostics, including real PostgreSQL foreign-key
  and unique-constraint violations carrying synthetic confidential text.
- `cargo test -p converge-server --lib http::error::tests`: a passing test
  checks that raw internal causes reach neither HTTP responses nor logs.
- Check both API and embedded builds; run Clippy and formatting checks.
- `env -u NO_COLOR trunk build --features api --locked` builds the actual WASM bundle.
- `npm run test:ui`: 12 passing Playwright tests, each using an isolated browser
  context and intercepted API requests with synthetic fixtures.
  `scripts/ui-errors.spec.js` invokes `scripts/check-ui-errors.js`.
  Its 35 checks exercise server rejection, server/network failures, pending guards,
  input preservation, successful retries, late responses, concurrent notices,
  sensitive token handling, and a 390×844 viewport. It also fails on browser
  exceptions or any application call to console log/info/debug/warn/error/etc.
  Browser-generated network diagnostics are distinct from application logging.
- `scripts/ui-interactions.spec.js` adds 11 regression tests for concurrent
  member actions, refreshed row data, disabled appearance, dismissed deletion,
  preserved navigation/group selection, invalid roster loads, repeated and
  independent notifications, folding/expansion, modal access, pointer hit
  testing, and scrolling at a narrow viewport. These also reject application
  console calls and browser exceptions. Real screen-reader announcement
  behavior has not been tested.

To repeat the browser checks, build the API bundle from `crates/converge-web`,
then install and run the browser runner from the repository root:

```sh
npm ci
npx playwright install chromium
npm run test:ui
```

The runner starts and stops its own static server on port 8086. Locally it can
use Chrome installed in `/Applications`, or the executable specified by
`PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH`; CI uses Playwright's Chromium. Failure
screenshots and traces are saved under `target/playwright`. Checks use no
production memberships, credentials, or external telemetry.
