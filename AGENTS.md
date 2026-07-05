# Personal-Assistant-Waifu — Frontend Architecture

Rules are ported from the Flutter/MobX naturaleza-mexicana app and adapted to a Tauri + Vite + React + TypeScript + MobX stack. This file is the single source of truth for the React frontend; Rust backend rules live in `src-tauri/` and are out of scope here.

Don't document what exists - document where to find it and how to work with it.

## Docker — CRITICAL

**The entire project runs inside Docker. The host machine must remain isolated from all package managers and node_modules.**

### Hard blocks (NEVER do these on the host)

- **NEVER run `bun install`**, `npm install`, `pnpm install`, `yarn install`, or any package manager install command on the host.
- **NEVER run `bun add`**, `npm add`, or any package-adding command on the host.
- **NEVER run `bun run dev`**, `bun run build`, `bun run lint`, `bun run typecheck`, or any script from `package.json` on the host.
- **NEVER run `cargo build`**, `cargo check`, `cargo run`, or any Rust build command on the host.
- **NEVER invoke `tauri`**, `bunx tauri`, `cargo tauri`, or any Tauri CLI command on the host.
- **NEVER create, read, or reference `node_modules/` from the host.** If you need to inspect a dependency, use `make dev-shell` and read it inside the container.

### How to work

The Docker image installs all system deps (webkit2gtk, gtk3, alsa, gstreamer, etc.), Rust toolchain, Bun, and Tauri CLI. The project directory is mounted at `/app` inside the container.

`node_modules/` is isolated from the host via a named Docker volume (`waifu-node-modules`). The volume shadows `/app/client/node_modules` inside the container, so `bun install` writes to the Docker volume, not the host filesystem. If `node_modules/` ever appears on the host, delete it immediately — it means the volume mount is broken or was bypassed.

```bash
# Build the Docker image (do this first, and again after Dockerfile.dev changes)
make dev-build

# Run the full Tauri app (Vite + Rust backend + webview)
make dev-run

# Open an interactive shell inside the container (for installing deps, running
# lint/typecheck, inspecting files, etc.)
make dev-shell
```

### If node_modules appears on the host

This should never happen with the volume mount in place. If it does:

```bash
# Delete root-owned files via Docker (not sudo on the host):
docker run --rm -v $(pwd):/app:rw -w /app alpine:latest sh -c "rm -rf /app/client/node_modules"
```

### Installing packages / adding dependencies

Always do this inside the container:

```bash
make dev-shell
# then inside the container:
cd /app/client && bun add <package>
# or for Rust crates:
cd /app/src-tauri && cargo add <crate>
```

### Running lint, typecheck, tests

```bash
make dev-shell
# then inside the container:
cd /app/client && bun run typecheck
cd /app/client && bun run lint
```

### CJK fonts

The app bundles Noto Sans SC as a web font via `@fontsource/noto-sans-sc` (imported in `client/src/main.tsx`). The Docker container has no CJK system fonts — the web font is the sole source of CJK glyphs. When adding new font weights or subsets, do it inside the container via `make dev-shell`.

## Architecture Rules

```
[State]        ← Components observe via mobx-react-lite observer()
  [domain]     ← Business data
  [view]       ← UI data: route, form fields, selections
[Workflows]    ← Components call fire-and-forget, workflows mutate any state
```

- **Fail fast** — Never swallow errors or try to recover from undefined states. Let it throw. The root Error Boundary catches it.
- **State is data, Workflows are actions** — If it HOLDS something → `state/`. If it DOES something → `workflows/`. No in-between.
- **State stores are pure data containers** — `makeAutoObservable`-decorated classes with observable fields, computed getters, simple setters and `reset()`/`clear()` methods. No Tauri invokes, no business logic, no cross-store coordination.
- **Workflows are atomic use cases** — They call `invoke()` / the Tauri event adapter, mutate any state (`state.domain.*`, `state.view.*`), call `state.view.routing.goTo*()`, and call other workflows. One workflow = one complete user-initiated flow.
- **Components call workflows fire-and-forget** — Components call `workflows.*.method()` without `await`. The workflow updates state, the component reacts via `observer()`. Components never coordinate domain stores or handle invoke responses.
- **Components never call state directly for actions** — Components read state for display (`state.domain.call.callState`), but all actions go through workflows. Exception: pure view state setters like `state.view.login.setEmail()`.
- **No try/catch or try/finally in workflows** — Let errors bubble up to the Error Boundary. Logic checks on invoke return values are fine. Don't wrap code in `try/finally` to set `isLoading = false` — just set it after the block.
- **No state in components** — Components must be function components wrapped in `observer()`. No `useState`/`useReducer` for app state. Exception: ephemeral UI-only state like hover effects, the wake-bar `--h` CSS variable, and animations.
- **Loading states belong in domain state** — `isLoading`/`callState` flags go in `state/domain/`, read by components directly.
- **Clear state on open, not on leave** — Workflows reset/clear relevant state when opening a view, never when leaving. Assume all views are dirty before opening them.
- **List items need their own observer** — Observable reads inside `array.map()` / `children` callbacks rendered in lists execute outside the parent's tracking. Wrap each list item component in `observer()` (or make the item its own `observer()` component) so it re-renders independently when its slice of state changes.

### File structure
```
client/src/
  main.tsx                ← mount + Tauri event adapter → workflows
  App.tsx                 ← observer-wrapped root + ErrorBoundary + <Routing>
  components/              ← observer()-wrapped presentational components only
    StatusBar.tsx
    WakeHero.tsx
    Messages.tsx
    Controls.tsx
    fields/
      AppTextField.tsx    ← shared form text field (see Form Validation)
  routing.tsx             ← <Routing> switch over state.view.routing.route
  stores/
    stores.ts             ← state + workflows singletons
    state/
      domain/             ← Business data stores
      view/               ← UI state stores
    workflows/            ← One workflow per use case
```

### Adding new features
1. **New state**: Add to `state/domain/` or `state/view/`, register in `stores.ts`
2. **New workflow**: Add to `workflows/`, register in `stores.ts`
3. **New route**: Add a variant to `Route` in `state/view/routing.ts`, add a `goTo*()` action to `RoutingState`, add a case in `routing.tsx`

## Store shape (mirrors naturaleza's `stores.dart`)

`stores.ts` exports two singletons:
```ts
export const state = new AppState();        // { domain, view }
export const workflows = new Workflows();    // one field per workflow
```
`AppState` holds `domain: DomainState` and `view: ViewState`; each of those is a plain object aggregating the store instances. Import `state`/`workflows` directly — no Provider, no context.

## MobX conventions (no codegen)

- Use `makeAutoObservable(this)` in every store constructor. No decorator plugin, no Babel config, no `.g.ts` codegen step.
- Observable fields → plain class fields. Computed values → `get` getters. Actions/mutations → plain methods. Setters → plain `set field(val)` methods.
- View state that depends on other state uses `autorun`/`reaction` (see Form Validation). Never hand-compute derived UI flags inside components.

## Tauri Contract (the only API surface)

The Rust backend (`src-tauri/`) talks to the frontend through Tauri events and commands. This replaces naturaleza's HTTP `res` field scheme.

- **Events (Rust → frontend, listen via `listen()`)** — adapter in `main.tsx` fans these out to workflows:
  - `wake_state`: payload is either a string or `{ state, score?, msg? }` with states `listening | hearing | processing | fired | rejected | error`. Adapter fails fast on unknown states.
  - `wake_rms` (0..1), `wake_vad` (0..1): written to `state.domain.wake.lastRms`/`lastVad`.
  - `qwen_state`: string `idle | wake_detected | connecting | connected | listening | speaking | disconnected`.
  - `user_transcript`, `qwen_transcript` (streaming delta), `qwen_response` (final), `qwen_error`.
- **Commands (frontend → Rust, via `invoke()`)** — only workflows call these: `start_call` (manual call start; wake word is deferred behind `KASSANDRA_WAKE_ENABLED`, default false), `toggle_mute` (returns bool), `end_call`, `console_log` (error logging).
- **Fail fast at the adapter boundary**: if a `wake_state`/`qwen_state` payload doesn't match the expected shape or enum, throw. Workflows should never see a malformed event.
- The event adapter in `main.tsx` is the one place `listen()` is called. Components/workflows never register listeners directly.

## Routing (preemptively scaffolded)

Mirror of naturaleza's `RoutingState`. A `Route` discriminated union lives in `state/view/routing.ts`; `RoutingState` holds `@observable route: Route` and one `goTo*()` action per route. The `<Routing>` component in `routing.tsx` is an `observer()` that switches over `state.view.routing.route` to render the matching screen. Only one route today (`chat`); the scaffold exists so new screens (avatar, memory editor, game mode, browser control) drop in without reinventing the pattern.

## Form Validation (preemptively scaffolded)

Reaction-driven validation in view state stores, mirroring naturaleza's client-app rules. Reactions (`reaction((_) => field, validateField)`) watch field changes and populate observable error fields. Components read computed booleans — zero validation logic in components.

Pattern for form view states:
- **Constructor**: Set up `reaction((_) => field, validateField)` for each validated field
- **Error fields**: `fieldError: string | null` — set by validator methods, NOT computed
- **Validators**: `validateField(value: string): void` — return early on empty values to avoid stale errors on `clearForm()`
- **Display gates**: `get showFieldError(): boolean { return fieldError != null && field.length > 0 }`
- **Form validity**: `get formHasErrors(): boolean { return isFormTouched && (error1 != null || ...) }`
- **Button state**: `get submitBtnDisabled(): boolean { return isLoading || formHasErrors || anyFieldEmpty }`
- **Setters**: Set `isFormTouched = true` and clear `apiErrorMessage`
- **API errors**: Workflows write API-level errors to `apiErrorMessage` in the view state, not domain state
- **Clearing**: `clearForm()` resets all fields, errors, and increments `formResetCount`. Components use `key={form.formResetCount}` on the form container to force `<input>` recreation when the form stays visible after clearing
- **All form text inputs use `<AppTextField>`** (`components/fields/AppTextField.tsx`) so required-feedback and maxLength capping behave identically everywhere. Pass `required` and `maxLength` props; ephemeral focus/touched state is owned by the component.

## Routing/length cap (mirrors server-client length asymmetry)

No server yet. When a persistence layer arrives: server caps must be looser than client `AppConstants.<field>MaxLength`; client caps live in `client/src/util/constants.ts`. Today the scaffold just needs the convention documented; no length constants exist yet.

## Code Generation

No codegen step. MobX's `makeAutoObservable` replaces Flutter's `build_runner` `.g.dart` generation. If decorator-based stores are ever introduced, add a codegen command here — don't add decorators silently.

## Error Handling

- **Root Error Boundary** (`components/ErrorBoundary.tsx`): the React equivalent of naturaleza's `GlobalErrorHandler`. Catches render errors and unhandled throws from workflows via its `componentDidCatch`, writes to `state.view.error`, and invokes `console_log` to forward to the Rust side (analog of `/user/client-logs`). Renders the error screen from `state.view.error`.
- **`ErrorViewState`** (`state/view/error.ts`): holds current error + message, used by the boundary and any error page component.
- **No try/catch in workflows** — Workflows throw on unexpected Tauri payloads; the boundary turns the throw into a crash page. The only exception is the adapter, which may `catch` solely to route into `state.view.error` + `console_log` if a throw would otherwise escape React's render cycle (e.g. in an event listener fire-and-forget). Re-throws are preferred over swallowing.