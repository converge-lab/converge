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
domain explanations, while future diagnostics must preserve useful technical
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
- Preserving drafts through a complete sign-in/reload flow and auditing
  permissions of all displayed actions.

## Verification

- `cargo test -p converge-web --all-features --lib`: 19 passing tests cover safe error text,
  case-preserving handles, independent notification lifetimes, and failures
  after the form's reactive owner is disposed. A separate regression covers
  group deletion after an API response, without component context.
- Check both API and embedded builds; run Clippy and formatting checks.
- `env -u NO_COLOR trunk build --features api` builds the actual WASM bundle.
- `scripts/check-ui-errors.js` is a Playwright page function. It creates an
  isolated browser context and intercepts API requests with synthetic fixtures.
  Its 35 checks exercise server rejection, server/network failures, pending guards,
  input preservation, successful retries, late responses, concurrent notices,
  sensitive token handling, and a 390×844 viewport. It also fails on browser
  exceptions or any application call to console log/info/debug/warn/error/etc.
  Browser-generated network diagnostics are distinct from application logging.

To repeat the browser checks, build the API bundle and serve it locally from
the repository root:

```sh
python3 -m http.server 8086 --bind 127.0.0.1 --directory crates/converge-web/dist
```

Run `scripts/check-ui-errors.js` as the `filename` argument to Playwright's
`browser_run_code_unsafe`, or evaluate the file as a function and invoke it
with a Playwright `page` from another runner. The checks are not wired into CI.
They use no production memberships, credentials, or external telemetry.
