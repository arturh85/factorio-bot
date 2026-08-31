# shadcn-vue + reka-ui + Tailwind v4 Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove PrimeVue, PrimeIcons and the PrimeVue-era SCSS layout shell from the Vue SPA entirely, replacing them with shadcn-vue-style components written against reka-ui and a Tailwind v4 design-token theme, so every page renders from one styling system with Preflight on.

**Architecture:** Three layers. (1) `app/src/lib/utils.ts` + `app/src/assets/tailwind.css` hold the design tokens and the `cn()` class merger — the whole theming surface. (2) `app/src/components/ui/` holds small, hand-written, individually-tested components (Button, Card, Input, Textarea, Checkbox, Slider, Label, Toggle, Toaster, TreeView), each wrapping a reka-ui primitive where one exists and plain semantic HTML where one does not. (3) Pages and the layout shell (`App.vue`, `AppTopbar.vue`, `AppMenu.vue`, `AppFooter.vue`) consume those components with Tailwind utilities inline; the `assets/layout/**` SCSS tree and `AppConfig.vue` are deleted. Migration order is one component family at a time, each task swapping its own consumers so the app renders correctly after every commit; Preflight is turned on only in the second-to-last task, when nothing depends on browser-default or SCSS bare-element styling any more.

**Tech Stack:** Vue 3.5, Pinia 4, vue-router 5 (hash history), Vite 8, vitest 4 + @vue/test-utils + jsdom, TypeScript 5.9, ESLint 10 flat config, Tailwind v4, reka-ui 2, lucide-vue-next, class-variance-authority, clsx, tailwind-merge, pnpm 10.34.5.

**Spec:** `.superpowers/ui-inventory.md` (the pre-migration audit: route table, PrimeVue usage counts, SCSS shell structure, dead-PrimeFlex sweep, hardest-parts list). This plan argues from that audit; read it alongside this plan. The decision to replace PrimeVue with shadcn-vue + reka-ui + Tailwind v4, **including rebuilding the SCSS layout shell in Tailwind**, is a settled user decision. It is not open for relitigation, and no task here may propose keeping PrimeVue.

**Predecessor:** `docs/superpowers/plans/2026-08-30-frontend-transport-swap.md` (plan 5). **Plan 5 must be complete through its Task 12 before Task 1 of this plan starts.** This plan edits `SettingsPage.vue`, `ScriptPage.vue`, `App.vue` and `main.ts`, all of which plan 5's Tasks 10-12 rewrite; running them concurrently guarantees conflicts. Everything below assumes the post-plan-5 state: no Tauri anywhere in `app/`, no `restapiStore.ts`, no `configure-ynetwork.js`, `SettingsPage.vue` with plain text inputs instead of native file pickers, `ScriptPage.vue` with an `onUnmounted` hook calling `scriptStore.stopWatching()`.

---

## Global Constraints

### Derive the constraint from the thing; never restate it

**A check written as a list of expected names is a mirror of the code, and mirrors cannot fail.** They stay green when something is *added* — which is the drift they exist to catch. This was found independently in this project's TypeScript and its Lua within an hour of each other, so treat it as the default failure mode of any test that enumerates.

Three real instances:

- `app/src/api/types.ts` — the file the whole OpenAPI contract seam exists to protect — was an input to **no assertion**. Renaming a field there alone left `pnpm run lint` clean and 158 tests passing, because the contract spec compared the snapshot against its *own* hand-written table. A developer following the failure messages would have edited that table, gone green, and shipped a store reading `undefined`.
- A Lua surface test asserted six named functions exist and four named ones are gone — green forever if a seventh appeared.
- A predicate validator checked keys against a hand-written list of step fields, so adding a field left `count{ new_field = x }` raising "unknown key" for a field visibly present on the step, with the suite green.

**The fix is always the same: make the compiler derive it.** `Record<keyof Props, …>` forces every declared key present and the excess-property check rejects extras, so `tsc` fails in *both* directions. `keyof paths` on a generated OpenAPI type means a wrapper naming a route the server does not publish stops compiling. Where a type cannot express it, compare **sets** in both directions rather than asserting membership.

A test can be satisfied by editing the mirror. A type cannot.

**For this plan specifically:** every component task is a place to write "assert these props/slots/emits exist". Do not. Type the test's expectations against the component's own prop types, or compare the rendered attribute set against the declared one. A component spec that asserts a fixed list of classes or props is a mirror with extra steps.



Every task's requirements implicitly include this section.

- **The package manager is pnpm 10.34.5.** yarn was removed from this repo; `yarn <anything>` is wrong here and will either fail or corrupt the lockfile.
- Vite 8, vitest 4, TypeScript 5.9, ESLint 10 flat config at `app/eslint.config.mjs`.
- **The ESLint config bans double quotes and trailing commas** (`quotes: ['error', 'single']`, `comma-dangle: ['error', 'never']`). Every TypeScript and Vue snippet in this plan is already written that way; keep it that way. This is also why vendored third-party component source is a poor fit here — see "Decision: hand-written, not CLI-vendored" below.
- Frontend commands run from `app/`: `pnpm run lint` (tsc + vue-tsc + eslint), `pnpm run test:coverage`, `pnpm run build:web`.
- Every command needs the Nix devShell: `nix develop --command bash -c 'eval "$(mise env -s bash)"; <command>'`
- **`git diff` in this repo lies to greps.** `diff.external` is difftastic, so `git diff | grep "^+"` matches nothing and reads identically to a clean result. Pass `--no-ext-diff` to every `git diff`. `git show` and `git log` are unaffected.
- **Never a bare `git commit`.** Always `git commit --only <explicit paths>`. Another session works in this checkout and a bare commit would sweep up its files.
- **Do not run `just fix`** — it rewrites files workspace-wide.
- Commits go to `master`; no feature branches. Conventional-commit subjects.
- **Do not touch `crates/`** in any task of this plan. This is a frontend-only plan. The DTOs it reads live in `app/src/api/types.ts`, which is hand-written and pinned to the server's published OpenAPI spec by `app/src/api/openapi.contract.spec.ts` — changing a declaration there without the server changing fails `tsc`.

---

## The standard this project holds

> **Delete the fix, demand a named failure.**

Eleven tests in this project have turned out unable to fail. Two real ones, worth recognising by shape:

- a test asserting `toContain('502')` against a fixture that happened to contain the substring "502" somewhere else — it passed with the feature deleted;
- a URL-escaping test using the id `'7'`, where `encodeURIComponent('7') === '7'`, so the assertion held with the escaping removed.

UI work makes this worse, not better. A component test that mounts a component and asserts "it did not throw" is exactly the same shape, and so is `expect(wrapper.html()).toContain('div')`. Before you accept a test as done, name the deletion it would catch: *"if I delete X, this test fails with Y."* If you cannot name one, the test is decoration.

Concretely, for the tasks below:

| Kind of component | What a real assertion looks like |
|---|---|
| Class-merging (`cn`, Button variants) | `cn('p-2', 'p-4') === 'p-4'` — plain concatenation gives `'p-2 p-4'` and fails. Asserting only `toContain('p-4')` would pass either way and is banned. |
| Model wrappers (Input, Textarea, Checkbox, Toggle, Slider) | Drive the DOM (`trigger('input')`, `trigger('click')`) and assert the **emitted payload**, plus assert the inverse direction (prop in → rendered attribute out). Deleting the `defineModel` wiring must fail the test. |
| Adapters (Slider's `number` ↔ `number[]`) | Assert the exact shape crossing the boundary: `findComponent(SliderRoot).props('modelValue')` is `[4]`, and an emitted `[9]` re-emits as `9`. Deleting the `value[0]` unwrap re-emits `[9]` and fails. |
| Lists (Toaster, AppMenu, Dashboard, TreeView) | Assert **counts that vary with input** and the **text of a specific item**, never merely "renders something". Three menu entries in, three links out, and the third link's `to` is `/script`. |
| Pages | Assert the store call the interaction produces (`expect(spy).toHaveBeenCalledWith('abc')`) and the rendered consequence of a store state change. |
| Deletions (routes, PrimeFlex classes) | Assert the absence *by a route that no longer resolves* or `expect(html).not.toContain('p-grid')`. An absence assertion is only real if it fails on the pre-change code — check that it does. |

Every task below states the expected failure text for its red step. If the red step passes, stop: the test is wrong, not the code.

The frontend's tests today are plan 5's `src/api/` and `src/store/` specs (251 of them; the placeholder `dummy.spec.ts` was deleted by plan 5's Task 16). **This plan does not start a general testing initiative** — it does not add tests for `Editor.vue`, `GanttChart.vue`, the router beyond route existence, or plan 5's transport code. It requires only that every component *this plan creates* is genuinely covered, enforced by extending plan 5's coverage gate to `src/components/ui/**`, `src/composables/**` and `src/lib/**`.

---

## Decision: hand-written, not CLI-vendored

**Write the components by hand into `app/src/components/ui/`, against reka-ui primitives, using shadcn-vue's published class strings as the visual reference — do not run the shadcn-vue CLI.** The CLI's value is the maintained upstream source, and that value does not survive contact here: its output is double-quoted with trailing commas (two ESLint errors per file under `app/eslint.config.mjs`), it expects a `components.json` plus its own `@/lib/utils` and CSS-variable naming conventions that would fight the token names this plan derives from the existing palette, and it would install ten components where nine are needed. The nine files below are 20-45 lines each.

---

## Routes treated as dead, and deleted in Task 2

So that nobody later mistakes a deleted placeholder for lost work, here is the complete list, verified in `app/src/router.ts` and `app/src/App.vue`'s `menu` ref:

| Route | Component | Verdict |
|---|---|---|
| `/` | `pages/Dashboard.vue` | **Live**, in the menu. Kept, rewritten in Task 10. |
| `/settings` | `pages/SettingsPage.vue` | **Live**, in the menu. Kept, rewritten in Task 6. |
| `/rcon` | `pages/RconPage.vue` | **Live**, in the menu. Kept, rewritten in Task 7. |
| `/script` | `pages/ScriptPage.vue` | **Live**, in the menu. Kept, rewritten in Tasks 8-9. |
| `/tasks` | `pages/TasksPage.vue` → `components/GanttChart.vue` | **In the menu but a stub** (`<div>TODO</div>`). Kept — it is the placeholder for planner output and is menu-reachable. Markup restyled in Task 10; `GanttChart.vue` is left as the stub it is. |
| `/empty` | `pages/EmptyPage.vue` | **Dead.** Deleted with the component. |
| `/factorioMods` | `pages/EmptyPage.vue` | **Dead.** Deleted. |
| `/restApiDocss` | `pages/EmptyPage.vue` | **Dead.** Deleted (note the typo'd name — nothing links to it; Swagger UI is linked directly from SettingsPage). |
| `/luaApiDocss` | `pages/EmptyPage.vue` | **Dead.** Deleted. |
| `/workspace` | `pages/EmptyPage.vue` | **Dead.** Deleted. |
| `/instances` | `pages/GameInstances.vue` | **Dead.** Static unbound text, no menu entry. Deleted with the component. |

Nothing of substance is lost: `EmptyPage.vue` is the stock "place your custom content" card and `GameInstances.vue` is a heading plus the sentence "List of Setup Instances:" with no data binding. Deleting them also removes four of the nine files carrying dead PrimeFlex classes.

## Component decisions made once, here

- **`AppConfig.vue` is deleted, not ported** (Task 11). Two of its four controls (Input Style, Ripple Effect) are PrimeVue-only concepts whose handlers are already empty functions — they do nothing today. The other two (menu type static/overlay, menu colour dark/light) are theme-template leftovers; this redesign fixes one sidebar behaviour (pinned on desktop, overlay on mobile) and one sidebar palette. This is why `RadioButton` and `ToggleSwitch` get no replacement component: after Task 11 nothing uses them. See the self-review at the bottom.
- **`AppSubmenu.vue` is deleted and folded into `AppMenu.vue`** (Task 11). No menu entry has ever had `items`, so the recursive submenu machinery, its transition classes and the `v-ripple` directive usage are dead weight.
- **The `Tree` rebuild is hand-written, not reka-ui's `TreeRoot`** (Task 9). Correcting the audit: reka-ui **does** ship a Tree (`TreeRoot`/`TreeItem`, `getKey`/`getChildren`, `flattenItems`). It is nevertheless the wrong fit — it derives an item's expandability from materialised children, and its docs specify `getChildren` must return `undefined` for a childless node, while our directories have *no children in memory until after the user expands them* (`GET /api/v1/scripts?path=` lists one directory at a time). Modelling lazy directories through it needs placeholder child nodes. A recursive `TreeView.vue` with an explicit `expandedKeys` array and a pure `mergeNodes()` function is ~60 lines, has no such ambiguity, and is directly unit-testable.
- **Toast is hand-written** (Task 3), not reka-ui's Toast. The app's five call sites use an imperative global (`toast.add({severity, summary, detail, life})`) with no component context; a module-level reactive queue reproduces that exactly, in 50 lines, and tests deterministically under `vi.useFakeTimers()`. As a bonus it works outside `setup()`, which PrimeVue's `useToast()` did not.
- **`Splitter` uses reka-ui** (Task 8): `SplitterGroup` + `SplitterPanel` + `SplitterResizeHandle`, with `auto-save-id` replacing PrimeVue's `stateKey`/`stateStorage="local"` (verified: `autoSaveId` persists the layout to `localStorage`; `direction` is a **required** prop).
- **Icons move from the `primeicons` font to `lucide-vue-next`** (Tasks 5, 9, 11). Ten glyphs are in use out of ~1600 in the font.

---

## File Structure

| File | Responsibility |
|---|---|
| `app/src/lib/utils.ts` (create) | `cn()` — clsx + tailwind-merge. The only class-merging helper. |
| `app/src/assets/tailwind.css` (modify, Tasks 1 and 11) | The whole theme: `@theme` design tokens in Task 1; Preflight import and the `@layer base` element defaults in Task 11. |
| `app/src/components/ui/button-variants.ts` (create) | `buttonVariants` cva definition, separate from the SFC because an SFC cannot export a second binding. |
| `app/src/components/ui/Button.vue` (create) | Button. Variants: primary / success / danger / ghost. |
| `app/src/components/ui/Card.vue` (create) | The white panel every page uses; replaces the SCSS `.card`. |
| `app/src/components/ui/Input.vue` (create) | Single-line text/number input with an `invalid` state. |
| `app/src/components/ui/Textarea.vue` (create) | Multi-line input. |
| `app/src/components/ui/Label.vue` (create) | `<label>` with the form type scale. |
| `app/src/components/ui/Checkbox.vue` (create) | reka-ui `CheckboxRoot`/`CheckboxIndicator` + associated label. |
| `app/src/components/ui/Slider.vue` (create) | reka-ui `SliderRoot`; adapts `number` ↔ reka's `number[]`. |
| `app/src/components/ui/Toggle.vue` (create) | Two-state button with `aria-pressed`; replaces PrimeVue `ToggleButton`. |
| `app/src/components/ui/Toaster.vue` (create) | Renders the toast queue; mounted once in `App.vue`. |
| `app/src/components/ui/tree/mergeNodes.ts` (create) | Pure: splice a freshly listed directory's children into the tree by key. |
| `app/src/components/ui/tree/TreeView.vue` (create) | Recursive, lazy, single-select disclosure tree with `role="tree"` semantics. |
| `app/src/composables/useToast.ts` (create) | `add`/`remove`/`clear` over a module-level reactive queue, plus `toastMessages`. |
| `app/src/components/ProcessControl.vue` (modify, Tasks 3-5) | Uses the new Button, Toggle and toast. |
| `app/src/components/ScriptTree.vue` (modify, Task 9) | Owns lazy loading and selection; renders `TreeView`. |
| `app/src/pages/SettingsPage.vue` (modify, Task 6) | Rewritten markup: Card/Label/Input/Checkbox/Slider, 30 dead PrimeFlex classes gone. |
| `app/src/pages/RconPage.vue` (modify, Tasks 4 and 7) | Buttons then Textarea; PrimeFlex wrapper gone. |
| `app/src/pages/ScriptPage.vue` (modify, Tasks 4, 8) | Buttons then reka Splitter; PrimeFlex wrapper gone. |
| `app/src/pages/Dashboard.vue`, `app/src/pages/TasksPage.vue` (modify, Task 10) | Tailwind grid; PrimeFlex classes gone. |
| `app/src/router.ts` (modify, Task 2) | Five routes instead of eleven. |
| `app/src/pages/EmptyPage.vue`, `app/src/pages/GameInstances.vue` (delete, Task 2) | Placeholders. |
| `app/src/App.vue` (modify, Tasks 3 and 11) | Toaster mount; then the Tailwind shell. |
| `app/src/AppTopbar.vue`, `app/src/AppMenu.vue`, `app/src/AppFooter.vue` (modify, Task 11) | Tailwind shell parts. |
| `app/src/AppSubmenu.vue`, `app/src/AppConfig.vue` (delete, Task 11) | Folded away / theme-template leftovers. |
| `app/src/assets/layout/**` (delete, Task 11) | The 16-file SCSS shell. |
| `app/src/models/dashboard.ts` (modify, Task 11) | `DashboardMenu` (12 optional fields, mostly unused) → `MenuEntry {label, icon, to}`. |
| `app/src/main.ts` (modify, Tasks 3 and 12) | Toast clear on navigation; then the PrimeVue plugin/theme/directive/icon imports go. |
| `app/vite.config.mts` (modify, Task 1) | Coverage `include` extended to the new directories. |
| `app/package.json` (modify, Tasks 1 and 12) | New deps in; `primevue`, `@primeuix/themes`, `primeicons`, `sass` out. |

---

## Task 1: Foundations — dependencies, tokens, `cn()`, component-test harness

**Files:**
- Modify: `app/package.json`
- Modify: `app/src/assets/tailwind.css`
- Modify: `app/vite.config.mts`
- Create: `app/src/lib/utils.ts`
- Create: `app/src/lib/utils.spec.ts`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `cn(...inputs: ClassValue[]): string` from `@/lib/utils`
  - Tailwind theme tokens usable as utilities: colours `surface card divider ink ink-muted link brand brand-dark brand-light focus success success-dark danger danger-dark warn sidebar sidebar-ink sidebar-active sidebar-active-ink sidebar-border route-active`; radius `card` (`rounded-card`); spacing `sidebar` (`w-sidebar`, `ml-sidebar`) and `topbar` (`h-topbar`, `top-topbar`)
  - runtime deps `reka-ui`, `lucide-vue-next`, `class-variance-authority`, `clsx`, `tailwind-merge`; dev deps `@vue/test-utils`, `jsdom`
  - coverage gate extended to `src/lib/**/*.ts`, `src/composables/**/*.ts`, `src/components/ui/**/*.{ts,vue}`

- [ ] **Step 1: Install the dependencies**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm add reka-ui@^2 lucide-vue-next@^0.5 class-variance-authority@^0.7 clsx@^2 tailwind-merge@^3'
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm add -D @vue/test-utils@^2 jsdom@^27'
```

`tailwind-merge` must be v3 or newer — v2 knows Tailwind v3's class names and would mis-merge v4 utilities.

- [ ] **Step 2: Write the failing test**

Create `app/src/lib/utils.spec.ts`:

```ts
import {describe, expect, it} from 'vitest';
import {cn} from './utils';

describe('cn', () => {
    it('lets the later padding utility win instead of emitting both', () => {
        // Plain concatenation yields 'p-2 p-4', where the winner depends on
        // the order Tailwind happens to emit the two rules in. This is the
        // whole reason tailwind-merge is a dependency.
        expect(cn('p-2', 'p-4')).toBe('p-4');
    });

    it('keeps utilities that do not conflict', () => {
        expect(cn('rounded-card', 'bg-card')).toBe('rounded-card bg-card');
    });

    it('drops falsy entries from a conditional class list', () => {
        expect(cn('bg-card', false, undefined, null, 'p-4')).toBe('bg-card p-4');
    });
});
```

- [ ] **Step 3: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/lib/utils.spec.ts'
```

Expected: FAIL — `Failed to resolve import "./utils"`.

- [ ] **Step 4: Write `cn()`**

Create `app/src/lib/utils.ts`:

```ts
import {clsx, type ClassValue} from 'clsx';
import {twMerge} from 'tailwind-merge';

/**
 * Join class names, letting the last conflicting Tailwind utility win.
 *
 * Components take a `class` prop and merge it over their own defaults with
 * this, so a caller writing `class="bg-success"` on a primary Button gets a
 * green button rather than two competing background rules whose winner
 * depends on stylesheet order.
 */
export function cn(...inputs: ClassValue[]): string {
    return twMerge(clsx(inputs));
}
```

- [ ] **Step 5: Run it and watch it pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/lib/utils.spec.ts'
```

Expected: 3 passed.

- [ ] **Step 6: Add the design tokens**

Replace the whole of `app/src/assets/tailwind.css` with:

```css
/* Tailwind v4, deliberately still without Preflight.
 *
 * Preflight is Tailwind's global reset. The layout shell in assets/layout/
 * still styles bare elements (headings, buttons, links) directly, so the reset
 * is turned on only once that shell is gone -- see the last task of
 * docs/superpowers/plans/2026-08-31-shadcn-ui-redesign.md. */
@layer theme, base, components, utilities;

@import "tailwindcss/theme.css" layer(theme);
@import "tailwindcss/utilities.css" layer(utilities);

/* The palette below is the one the app already had: every value is lifted
 * verbatim from assets/layout/_variables.scss, so turning the SCSS shell into
 * Tailwind utilities is a change of mechanism, not of appearance. */
@theme {
    /* Surfaces */
    --color-surface: #edf0f5;
    --color-card: #ffffff;
    --color-divider: #e3e3e3;

    /* Text */
    --color-ink: #333333;
    --color-ink-muted: #707070;
    --color-link: #2196f3;

    /* Brand: the topbar gradient runs brand -> brand-light */
    --color-brand: #0388e5;
    --color-brand-dark: #026fba;
    --color-brand-light: #07bdf4;
    --color-focus: #8dcdff;

    /* Status */
    --color-success: #20d077;
    --color-success-dark: #1aa75f;
    --color-danger: #ef6262;
    --color-danger-dark: #d84a4a;
    --color-warn: #f9c851;

    /* Sidebar */
    --color-sidebar: #47555e;
    --color-sidebar-ink: #ffffff;
    --color-sidebar-active: #2e3035;
    --color-sidebar-active-ink: #3aa6f1;
    --color-sidebar-border: rgba(52, 56, 65, 0.6);
    --color-route-active: #1fa1fc;

    /* Shell metrics */
    --radius-card: 3px;
    --spacing-sidebar: 250px;
    --spacing-topbar: 50px;
}
```

- [ ] **Step 7: Extend the coverage gate to the new directories**

In `app/vite.config.mts`, replace the `test` block (plan 5 Task 16 left it scoped to `src/api` and `src/store`) with:

```ts
    test: {
        // Node stays the default so the transport specs keep running against
        // node's fetch. Component specs opt into a DOM per file with a
        // `// @vitest-environment jsdom` docblock on line 1.
        environment: 'node',
        coverage: {
            reporter: ['html-spa', 'cobertura', 'text'],
            // The transport swap's blast radius plus the components this
            // redesign introduces. Pages and the layout shell stay out: they
            // were untested before and a threshold that includes them would
            // have to be lowered to meaninglessness to pass.
            include: [
                'src/api/**/*.ts',
                'src/store/**/*.ts',
                'src/lib/**/*.ts',
                'src/composables/**/*.ts',
                'src/components/ui/**/*.ts',
                'src/components/ui/**/*.vue'
            ],
            exclude: ['src/api/openapi.snapshot.json', '**/*.spec.ts'],
            thresholds: {
                lines: 90,
                functions: 90,
                statements: 90,
                branches: 80
            }
        }
    },
```

- [ ] **Step 8: Full check**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint && pnpm run test:coverage && pnpm run build:web'
```

Expected: lint clean, tests pass, coverage thresholds met, build succeeds. The build is the only thing that proves the `@theme` block parses.

- [ ] **Step 9: Commit**

```bash
git commit --only app/package.json app/pnpm-lock.yaml app/vite.config.mts app/src/assets/tailwind.css app/src/lib/utils.ts app/src/lib/utils.spec.ts -m "feat(app): add the tailwind design tokens and the ui component toolchain"
```

---

## Task 2: Delete the six dead routes and their two placeholder pages

Doing this first removes four of the nine dead-PrimeFlex files before anyone has to restyle them.

**Files:**
- Modify: `app/src/router.ts`
- Delete: `app/src/pages/EmptyPage.vue`, `app/src/pages/GameInstances.vue`
- Create: `app/src/router.spec.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `app/src/router.ts` default-exports a router whose only routes are `/`, `/settings`, `/script`, `/rcon`, `/tasks`. `pages/EmptyPage.vue` and `pages/GameInstances.vue` no longer exist and must not be imported by any later task.

- [ ] **Step 1: Write the failing test**

Create `app/src/router.spec.ts`:

```ts
import {describe, expect, it} from 'vitest';
import router from './router';

const paths = () => router.getRoutes().map(route => route.path).sort();

describe('router', () => {
    it('exposes exactly the five routes the menu links to', () => {
        expect(paths()).toEqual(['/', '/rcon', '/script', '/settings', '/tasks']);
    });

    it('does not resolve the deleted placeholder routes', () => {
        for (const path of ['/empty', '/factorioMods', '/restApiDocss', '/luaApiDocss', '/workspace', '/instances']) {
            expect(router.resolve(path).matched).toEqual([]);
        }
    });
});
```

- [ ] **Step 2: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/router.spec.ts'
```

Expected: FAIL on both cases — the first because the array has eleven entries, the second because `/empty` still matches `EmptyPage.vue`. If either passes here, the router file being imported is not the one you are about to edit.

- [ ] **Step 3: Shrink the router**

Replace the whole of `app/src/router.ts` with:

```ts
import {createRouter, createWebHashHistory} from 'vue-router';

// Six further routes existed until this commit: /empty, /factorioMods,
// /restApiDocss, /luaApiDocss and /workspace all rendered the same stock
// "Empty Page" placeholder, and /instances rendered a static card with no data
// binding. None was reachable from the menu. They are deleted rather than
// restyled; see docs/superpowers/plans/2026-08-31-shadcn-ui-redesign.md.
const routes = [
    {
        path: '/',
        name: 'dashboard',
        component: () => import('./pages/Dashboard.vue')
    },
    {
        path: '/settings',
        name: 'settings',
        component: () => import('./pages/SettingsPage.vue')
    },
    {
        path: '/script',
        name: 'script',
        component: () => import('./pages/ScriptPage.vue')
    },
    {
        path: '/rcon',
        name: 'rcon',
        component: () => import('./pages/RconPage.vue')
    },
    {
        path: '/tasks',
        name: 'tasks',
        component: () => import('./pages/TasksPage.vue')
    }
];

const router = createRouter({
    history: createWebHashHistory(),
    routes
});

export default router;
```

- [ ] **Step 4: Delete the placeholder pages**

```bash
git rm app/src/pages/EmptyPage.vue app/src/pages/GameInstances.vue
```

- [ ] **Step 5: Run the test and the lint**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/router.spec.ts && pnpm run lint'
```

Expected: 2 passed; lint clean (nothing imported the two deleted files — `App.vue`'s menu never linked them and the commented-out menu entries carry no imports).

- [ ] **Step 6: Commit**

```bash
git commit --only app/src/router.ts app/src/router.spec.ts app/src/pages/EmptyPage.vue app/src/pages/GameInstances.vue -m "refactor(app): delete the six unreachable placeholder routes"
```

---

## Task 3: Toast — a composable queue and a Toaster, replacing PrimeVue's

**Files:**
- Create: `app/src/composables/useToast.ts`
- Create: `app/src/composables/useToast.spec.ts`
- Create: `app/src/components/ui/Toaster.vue`
- Create: `app/src/components/ui/Toaster.spec.ts`
- Modify: `app/src/App.vue`, `app/src/main.ts`, `app/src/components/ProcessControl.vue`, `app/src/pages/Dashboard.vue`, `app/src/pages/RconPage.vue`, `app/src/pages/ScriptPage.vue`

**Interfaces:**
- Consumes: `cn` is *not* needed here.
- Produces, from `@/composables/useToast`:
  - `type ToastSeverity = 'success' | 'info' | 'warn' | 'error'`
  - `interface ToastOptions {severity?: ToastSeverity; summary: string; detail?: string; life?: number}`
  - `interface ToastMessage {id: number; severity: ToastSeverity; summary: string; detail: string | null}`
  - `const toastMessages: DeepReadonly<ToastMessage[]>` — the live queue, for `Toaster.vue`
  - `function useToast(): {add(options: ToastOptions): number; remove(id: number): void; clear(): void}`
- Produces: `app/src/components/ui/Toaster.vue`, a no-prop component mounted once.
- Note for later tasks: the five existing call sites keep their exact shape (`toast.add({severity, summary, detail, life})`); only the import path changes. The one call to PrimeVue's `removeAllGroups()` becomes `clear()`.

- [ ] **Step 1: Write the failing composable test**

Create `app/src/composables/useToast.spec.ts`:

```ts
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import {toastMessages, useToast} from './useToast';

beforeEach(() => {
    vi.useFakeTimers();
    useToast().clear();
});

afterEach(() => {
    useToast().clear();
    vi.useRealTimers();
});

describe('useToast', () => {
    it('keeps a message until its life expires, then drops it', () => {
        useToast().add({severity: 'success', summary: 'Started!', life: 1000});
        vi.advanceTimersByTime(999);
        expect(toastMessages.length).toBe(1);
        vi.advanceTimersByTime(1);
        expect(toastMessages.length).toBe(0);
    });

    it('defaults severity to info and detail to null', () => {
        useToast().add({summary: 'Info Message'});
        expect(toastMessages[0].severity).toBe('info');
        expect(toastMessages[0].detail).toBeNull();
    });

    it('keeps a message with life 0 on screen indefinitely', () => {
        useToast().add({summary: 'sticky', life: 0});
        vi.advanceTimersByTime(600000);
        expect(toastMessages.length).toBe(1);
    });

    it('removes only the message asked for', () => {
        const first = useToast().add({summary: 'first', life: 0});
        useToast().add({summary: 'second', life: 0});
        useToast().remove(first);
        expect(toastMessages.map(message => message.summary)).toEqual(['second']);
    });

    it('cancels pending timers on clear, so a later message is not dropped early', () => {
        useToast().add({summary: 'first', life: 1000});
        useToast().clear();
        vi.advanceTimersByTime(1000);
        useToast().add({summary: 'second', life: 5000});
        vi.advanceTimersByTime(1000);
        // Without clearTimeout in clear(), the first message's timer fires at
        // t=1000 and splices index 0 -- which by then is 'second'.
        expect(toastMessages.map(message => message.summary)).toEqual(['second']);
    });
});
```

- [ ] **Step 2: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/composables/useToast.spec.ts'
```

Expected: FAIL — `Failed to resolve import "./useToast"`.

- [ ] **Step 3: Write the composable**

Create `app/src/composables/useToast.ts`:

```ts
import {reactive, readonly} from 'vue';

export type ToastSeverity = 'success' | 'info' | 'warn' | 'error';

export interface ToastOptions {
    severity?: ToastSeverity;
    summary: string;
    detail?: string;
    /** Milliseconds before auto-dismissal. 0 keeps the message until removed. */
    life?: number;
}

export interface ToastMessage {
    id: number;
    severity: ToastSeverity;
    summary: string;
    detail: string | null;
}

/**
 * One queue for the whole app, deliberately module-level.
 *
 * PrimeVue's useToast() resolved a service through Vue's injection, which
 * meant it only worked inside setup(). This app calls it from route guards
 * and from async callbacks that outlive the component, so the queue lives in
 * the module and any caller anywhere gets the same one.
 */
const messages = reactive([] as ToastMessage[]);
const timers = new Map<number, ReturnType<typeof setTimeout>>();
let nextId = 1;

export const toastMessages = readonly(messages);

function remove(id: number): void {
    const timer = timers.get(id);
    if (timer !== undefined) {
        clearTimeout(timer);
        timers.delete(id);
    }
    const index = messages.findIndex(message => message.id === id);
    if (index !== -1) {
        messages.splice(index, 1);
    }
}

function add(options: ToastOptions): number {
    const id = nextId++;
    messages.push({
        id,
        severity: options.severity ?? 'info',
        summary: options.summary,
        detail: options.detail ?? null
    });
    const life = options.life ?? 5000;
    if (life > 0) {
        timers.set(id, setTimeout(() => remove(id), life));
    }
    return id;
}

function clear(): void {
    for (const timer of timers.values()) {
        clearTimeout(timer);
    }
    timers.clear();
    messages.splice(0, messages.length);
}

export function useToast() {
    return {add, remove, clear};
}
```

- [ ] **Step 4: Run it and watch it pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/composables/useToast.spec.ts'
```

Expected: 5 passed.

- [ ] **Step 5: Write the failing Toaster test**

Create `app/src/components/ui/Toaster.spec.ts`:

```ts
// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Toaster from './Toaster.vue';
import {useToast} from '@/composables/useToast';

beforeEach(() => useToast().clear());
afterEach(() => useToast().clear());

describe('Toaster', () => {
    it('renders one panel per queued message, newest last', async () => {
        const wrapper = mount(Toaster);
        useToast().add({summary: 'first', life: 0});
        useToast().add({summary: 'second', detail: 'with detail', life: 0});
        await wrapper.vm.$nextTick();

        const toasts = wrapper.findAll('[data-testid="toast"]');
        expect(toasts.length).toBe(2);
        expect(toasts[1].text()).toContain('with detail');
    });

    it('colours an error differently from a success', async () => {
        const wrapper = mount(Toaster);
        useToast().add({severity: 'error', summary: 'boom', life: 0});
        await wrapper.vm.$nextTick();
        expect(wrapper.find('[data-testid="toast"]').classes()).toContain('border-danger');
        expect(wrapper.find('[data-testid="toast"]').classes()).not.toContain('border-success');
    });

    it('drops the message whose dismiss button is pressed', async () => {
        const wrapper = mount(Toaster);
        useToast().add({summary: 'first', life: 0});
        useToast().add({summary: 'second', life: 0});
        await wrapper.vm.$nextTick();

        await wrapper.findAll('[aria-label="Dismiss"]')[0].trigger('click');
        const toasts = wrapper.findAll('[data-testid="toast"]');
        expect(toasts.length).toBe(1);
        expect(toasts[0].text()).toContain('second');
    });

    it('announces politely so a screen reader reads new messages', () => {
        const wrapper = mount(Toaster);
        expect(wrapper.attributes('aria-live')).toBe('polite');
    });
});
```

- [ ] **Step 6: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/Toaster.spec.ts'
```

Expected: FAIL — `Failed to resolve import "./Toaster.vue"`.

- [ ] **Step 7: Write the Toaster**

Create `app/src/components/ui/Toaster.vue`:

```vue
<script setup lang="ts">
import {X} from 'lucide-vue-next';
import {toastMessages, useToast, type ToastSeverity} from '@/composables/useToast';

const {remove} = useToast();

const severityClasses: Record<ToastSeverity, string> = {
    success: 'border-success',
    info: 'border-brand',
    warn: 'border-warn',
    error: 'border-danger'
};
</script>

<template>
  <div
    class="pointer-events-none fixed bottom-4 right-4 z-[1000] flex w-80 flex-col gap-2"
    role="status"
    aria-live="polite"
    data-testid="toaster">
    <div
      v-for="message in toastMessages"
      :key="message.id"
      class="pointer-events-auto flex items-start gap-2 rounded-card border-l-4 bg-card p-3 text-ink shadow-lg"
      :class="severityClasses[message.severity]"
      data-testid="toast">
      <div class="grow">
        <p class="font-semibold">{{ message.summary }}</p>
        <p v-if="message.detail" class="mt-1 text-ink-muted">{{ message.detail }}</p>
      </div>
      <button
        type="button"
        class="text-ink-muted transition-colors hover:text-ink"
        aria-label="Dismiss"
        @click="remove(message.id)">
        <X class="size-4"/>
      </button>
    </div>
  </div>
</template>
```

- [ ] **Step 8: Run it and watch it pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/Toaster.spec.ts'
```

Expected: 4 passed.

- [ ] **Step 9: Point the five call sites at the new composable**

In each of `app/src/components/ProcessControl.vue`, `app/src/pages/Dashboard.vue`, `app/src/pages/RconPage.vue` and `app/src/pages/ScriptPage.vue`, replace the import line

```ts
import {useToast} from 'primevue/usetoast';
```

(`ProcessControl.vue` spells it `import { useToast } from 'primevue/usetoast';` — same line, different spacing) with

```ts
import {useToast} from '@/composables/useToast';
```

No call site changes: `toast.add({severity, summary, detail, life})` is the same call.

- [ ] **Step 10: Mount the Toaster in App.vue and move the clear-on-navigate hook**

In `app/src/App.vue`:

1. Replace `import Toast from 'primevue/toast';` with `import Toaster from '@/components/ui/Toaster.vue';`
2. Replace `<Toast position="bottom-right"/>` with `<Toaster/>`
3. Delete the `import {onBeforeRouteLeave} from 'vue-router';` line and the whole `onBeforeRouteLeave(() => {...})` block (the one calling `toast.removeAllGroups()`). The replacement lives in `main.ts`, where it also covers the first navigation.
4. Delete the `<style lang="scss">` block at the bottom (`.p-toast.p-toast-bottom-right { z-index: 1000 }`) — it targets a PrimeVue class that no longer renders; the Toaster carries `z-1000` itself.
5. If `useToast` is now unused in `App.vue`'s script, keep it: plan 5's `onMounted` uses it for the "Cannot reach the factorio-bot server" toast. Only the import path changes, per Step 9's pattern — `App.vue` imports it too.

In `app/src/main.ts`, replace the existing guard

```ts
router.beforeEach(() => {
    window.scrollTo(0, 0);
});
```

with

```ts
router.beforeEach(() => {
    // Messages are about the page the user is leaving. Clearing here rather
    // than in a component hook also covers navigations that unmount nothing.
    useToast().clear();
    window.scrollTo(0, 0);
});
```

and add, next to the other local imports at the top of `main.ts`:

```ts
import {useToast} from './composables/useToast';
```

- [ ] **Step 11: Verify PrimeVue's toast is gone and nothing else broke**

```bash
grep -rn "primevue/usetoast\|removeAllGroups" app/src
grep -rn "primevue/toast" app/src
```

Expected: no output from the first; exactly one line from the second — `app/src/main.ts` still registers `primevue/toastservice`, which is harmless and goes with the rest of PrimeVue in Task 12.

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint && pnpm run test:coverage && pnpm run build:web'
```

Expected: all clean. `ToastService` stays registered in `main.ts` for now — it is harmless and Task 12 removes it with the rest of PrimeVue.

- [ ] **Step 12: Commit**

```bash
git commit --only app/src/composables/useToast.ts app/src/composables/useToast.spec.ts app/src/components/ui/Toaster.vue app/src/components/ui/Toaster.spec.ts app/src/App.vue app/src/main.ts app/src/components/ProcessControl.vue app/src/pages/Dashboard.vue app/src/pages/RconPage.vue app/src/pages/ScriptPage.vue -m "feat(app): replace primevue toast with a module-level toast queue"
```

---

## Task 4: Button

**Files:**
- Create: `app/src/components/ui/button-variants.ts`
- Create: `app/src/components/ui/Button.vue`
- Create: `app/src/components/ui/Button.spec.ts`
- Modify: `app/src/components/ProcessControl.vue`, `app/src/pages/RconPage.vue`, `app/src/pages/ScriptPage.vue`, `app/src/pages/SettingsPage.vue` (only if a `<Button>` still remains there after plan 5 — see Step 7)

**Interfaces:**
- Consumes: `cn` from `@/lib/utils` (Task 1).
- Produces:
  - `buttonVariants` (cva) and `type ButtonVariants = VariantProps<typeof buttonVariants>` from `@/components/ui/button-variants`
  - `@/components/ui/Button.vue` with props `{variant?: 'primary' | 'success' | 'danger' | 'ghost'; size?: 'default' | 'sm' | 'icon'; type?: 'button' | 'submit'; disabled?: boolean; class?: HTMLAttributes['class']}`, default slot for the label, native `click` passthrough. **There is no `label` prop** — PrimeVue's `label="Run"` becomes slot content.

- [ ] **Step 1: Write the failing test**

Create `app/src/components/ui/Button.spec.ts`:

```ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Button from './Button.vue';

describe('Button', () => {
    it('renders its slot inside a non-submitting button', () => {
        const wrapper = mount(Button, {slots: {default: 'Run'}});
        expect(wrapper.element.tagName).toBe('BUTTON');
        // Without an explicit type, a button inside a form submits it.
        expect(wrapper.attributes('type')).toBe('button');
        expect(wrapper.text()).toBe('Run');
    });

    it('paints the danger variant and not the primary one', () => {
        const wrapper = mount(Button, {props: {variant: 'danger'}, slots: {default: 'Stop'}});
        expect(wrapper.classes()).toContain('bg-danger');
        expect(wrapper.classes()).not.toContain('bg-brand');
    });

    it('lets a caller-supplied background override the variant background', () => {
        const wrapper = mount(Button, {props: {variant: 'primary', class: 'bg-success'}});
        // Concatenating instead of merging leaves both, and which one wins
        // then depends on Tailwind's emit order rather than on the caller.
        expect(wrapper.classes()).toContain('bg-success');
        expect(wrapper.classes()).not.toContain('bg-brand');
    });

    it('carries the disabled attribute and swallows the click when disabled', async () => {
        const wrapper = mount(Button, {props: {disabled: true}, slots: {default: 'Running ...'}});
        expect(wrapper.attributes('disabled')).toBeDefined();
        await wrapper.trigger('click');
        expect(wrapper.emitted('click')).toBeUndefined();
    });

    it('emits click when enabled', async () => {
        const wrapper = mount(Button, {slots: {default: 'Run'}});
        await wrapper.trigger('click');
        expect(wrapper.emitted('click')).toHaveLength(1);
    });
});
```

- [ ] **Step 2: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/Button.spec.ts'
```

Expected: FAIL — `Failed to resolve import "./Button.vue"`.

- [ ] **Step 3: Write the variants**

Create `app/src/components/ui/button-variants.ts`:

```ts
import {cva, type VariantProps} from 'class-variance-authority';

/**
 * Kept out of Button.vue because an SFC's <script setup> cannot export a
 * second binding, and pages occasionally want the class string on an <a>.
 */
export const buttonVariants = cva(
    'inline-flex cursor-pointer items-center justify-center gap-2 rounded-card font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus disabled:cursor-not-allowed disabled:opacity-60',
    {
        variants: {
            variant: {
                primary: 'bg-brand text-white hover:bg-brand-dark',
                success: 'bg-success text-white hover:bg-success-dark',
                danger: 'bg-danger text-white hover:bg-danger-dark',
                ghost: 'bg-transparent text-ink hover:bg-divider'
            },
            size: {
                default: 'h-9 px-4 text-sm',
                sm: 'h-8 px-3 text-xs',
                icon: 'size-9'
            }
        },
        defaultVariants: {
            variant: 'primary',
            size: 'default'
        }
    }
);

export type ButtonVariants = VariantProps<typeof buttonVariants>;
```

- [ ] **Step 4: Write the component**

Create `app/src/components/ui/Button.vue`:

```vue
<script setup lang="ts">
import {computed, type HTMLAttributes} from 'vue';
import {cn} from '@/lib/utils';
import {buttonVariants, type ButtonVariants} from './button-variants';

// `class` is declared as a prop on purpose: that takes it out of $attrs, so
// Vue does not also auto-merge it onto the root element and defeat cn().
const props = withDefaults(defineProps<{
  variant?: ButtonVariants['variant'];
  size?: ButtonVariants['size'];
  type?: 'button' | 'submit';
  disabled?: boolean;
  class?: HTMLAttributes['class'];
}>(), {
  variant: 'primary',
  size: 'default',
  type: 'button',
  disabled: false,
  class: undefined
});

const classes = computed(() => cn(buttonVariants({variant: props.variant, size: props.size}), props.class));
</script>

<template>
  <button :type="type" :disabled="disabled" :class="classes">
    <slot/>
  </button>
</template>
```

- [ ] **Step 5: Run it and watch it pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/Button.spec.ts'
```

Expected: 5 passed.

- [ ] **Step 6: Swap the buttons in `ProcessControl.vue`**

In `app/src/components/ProcessControl.vue`, replace `import Button from 'primevue/button';` with

```ts
import Button from '@/components/ui/Button.vue';
import {Check} from 'lucide-vue-next';
```

and replace the `<Button …>` element in the template (the one with `:icon`, `:label`, `:severity`) with:

```html
  <Button :variant="isStarted ? 'success' : 'danger'"
          :disabled="isStopping || isStarting"
          @click="isStarted ? stopInstances() : startInstances()">
    <Check v-if="isStarted" class="size-4" aria-hidden="true"/>
    {{ buttonLabel }}
  </Button>
```

Leave the `<ToggleButton>` element alone — Task 5 replaces it.

- [ ] **Step 7: Swap the buttons in `RconPage.vue`, `ScriptPage.vue` and `SettingsPage.vue`**

In `app/src/pages/RconPage.vue`, replace `import Button from 'primevue/button';` with `import Button from '@/components/ui/Button.vue';`, and rewrite the five buttons — PrimeVue's `label` prop becomes slot text:

```html
          <Button :disabled="isExecuting" @click="execute(command)">{{ isExecuting ? 'Running ...' : 'Run' }}</Button>
```

```html
        <Button variant="ghost" @click="execute('/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'stone-furnace\', 20)')">Cheat Furnaces</Button>
        <Button variant="ghost" @click="execute('/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'transport-belt\', 100)')">Cheat belts</Button>
        <Button variant="ghost" @click="execute('/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'burner-mining-drill\', 20)')">Cheat Drills</Button>
        <Button variant="ghost" @click="execute('/server-save')">Save</Button>
```

In `app/src/pages/ScriptPage.vue`, replace `import Button from 'primevue/button';` with `import Button from '@/components/ui/Button.vue';` and rewrite its one button as:

```html
          <Button :disabled="isExecuting" @click="execute()">{{ isExecuting ? 'Running ...' : 'Run' }}</Button>
```

In `app/src/pages/SettingsPage.vue`, check first:

```bash
grep -n "Button" app/src/pages/SettingsPage.vue
```

Plan 5's Task 11 deleted both "Select" buttons and the `Button` import with them, so expect **no output**. If a `<Button>` is still there, do the same swap; if only the import is left, delete the import.

- [ ] **Step 8: Verify and commit**

```bash
grep -rn "primevue/button" app/src
```

Expected: no output.

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint && pnpm run test:coverage'
```

Expected: clean.

```bash
git commit --only app/src/components/ui/button-variants.ts app/src/components/ui/Button.vue app/src/components/ui/Button.spec.ts app/src/components/ProcessControl.vue app/src/pages/RconPage.vue app/src/pages/ScriptPage.vue app/src/pages/SettingsPage.vue -m "feat(app): replace the primevue button with a tailwind button"
```

(Drop `app/src/pages/SettingsPage.vue` from the path list if Step 7 did not change it — `git commit --only` on an unchanged path is fine but the message should not claim a change.)

---

## Task 5: Toggle

**Files:**
- Create: `app/src/components/ui/Toggle.vue`
- Create: `app/src/components/ui/Toggle.spec.ts`
- Modify: `app/src/components/ProcessControl.vue`

**Interfaces:**
- Consumes: `cn` from `@/lib/utils` (Task 1).
- Produces: `@/components/ui/Toggle.vue` with `v-model` of type `boolean` (via `defineModel<boolean>({required: true})`), props `{onLabel: string; offLabel: string; class?: string}`, rendering a `<button type="button" aria-pressed>`.

A two-state button is fully expressible with `aria-pressed` on a native button, so this one does not need a reka-ui primitive; adding one would be more code and more indirection for the same semantics.

- [ ] **Step 1: Write the failing test**

Create `app/src/components/ui/Toggle.spec.ts`:

```ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Toggle from './Toggle.vue';

function mountToggle(modelValue: boolean) {
    return mount(Toggle, {props: {modelValue, onLabel: 'Recreate Level', offLabel: 'Use existing Level'}});
}

describe('Toggle', () => {
    it('shows the off label and reports aria-pressed false when unset', () => {
        const wrapper = mountToggle(false);
        expect(wrapper.text()).toBe('Use existing Level');
        expect(wrapper.attributes('aria-pressed')).toBe('false');
    });

    it('shows the on label and reports aria-pressed true when set', () => {
        const wrapper = mountToggle(true);
        expect(wrapper.text()).toBe('Recreate Level');
        expect(wrapper.attributes('aria-pressed')).toBe('true');
    });

    it('emits the flipped value when clicked', async () => {
        const wrapper = mountToggle(false);
        await wrapper.trigger('click');
        expect(wrapper.emitted('update:modelValue')).toEqual([[true]]);
    });

    it('emits false when clicked while set', async () => {
        const wrapper = mountToggle(true);
        await wrapper.trigger('click');
        expect(wrapper.emitted('update:modelValue')).toEqual([[false]]);
    });
});
```

- [ ] **Step 2: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/Toggle.spec.ts'
```

Expected: FAIL — `Failed to resolve import "./Toggle.vue"`.

- [ ] **Step 3: Write the component**

Create `app/src/components/ui/Toggle.vue`:

```vue
<script setup lang="ts">
import {computed} from 'vue';
import {Check, X} from 'lucide-vue-next';
import {cn} from '@/lib/utils';

const props = withDefaults(defineProps<{
  onLabel: string;
  offLabel: string;
  class?: string;
}>(), {
  class: undefined
});

const model = defineModel<boolean>({required: true});

const classes = computed(() => cn(
  'inline-flex h-9 cursor-pointer items-center gap-2 rounded-card border px-4 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus',
  model.value ? 'border-brand bg-brand text-white' : 'border-divider bg-card text-ink',
  props.class
));
</script>

<template>
  <button type="button" :aria-pressed="model" :class="classes" @click="model = !model">
    <Check v-if="model" class="size-4" aria-hidden="true"/>
    <X v-else class="size-4" aria-hidden="true"/>
    <span>{{ model ? onLabel : offLabel }}</span>
  </button>
</template>
```

- [ ] **Step 4: Run it and watch it pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/Toggle.spec.ts'
```

Expected: 4 passed.

- [ ] **Step 5: Swap it into `ProcessControl.vue`**

Replace `import ToggleButton from 'primevue/togglebutton';` with

```ts
import Toggle from '@/components/ui/Toggle.vue';
```

and replace the `<ToggleButton …/>` element with:

```html
  <Toggle v-model="recreateLevel" on-label="Recreate Level" off-label="Use existing Level"/>
```

`recreateLevel` is a `computed` with a setter over `appStore.getRecreateLevel` / `appStore.updateRecreateLevel`; its getter is typed `boolean | undefined`. If `vue-tsc` objects to binding it to a `boolean` model, narrow the getter in `ProcessControl.vue` — `get() { return appStore.getRecreateLevel === true }` — rather than loosening the component's prop type.

- [ ] **Step 6: Verify and commit**

```bash
grep -rn "primevue/togglebutton\|primevue/toggleswitch" app/src
```

Expected: only `app/src/AppConfig.vue` (the `toggleswitch` import), which Task 11 deletes.

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint && pnpm run test:coverage'
```

```bash
git commit --only app/src/components/ui/Toggle.vue app/src/components/ui/Toggle.spec.ts app/src/components/ProcessControl.vue -m "feat(app): replace the primevue togglebutton with an aria-pressed toggle"
```

---

## Task 6: Form controls and the Settings page

The single biggest and most error-prone file in the migration: eight fields, four control types, and thirty dead PrimeFlex classes that make it the worst-rendering page in the app today.

**Files:**
- Create: `app/src/components/ui/Card.vue`, `app/src/components/ui/Label.vue`, `app/src/components/ui/Input.vue`, `app/src/components/ui/Checkbox.vue`, `app/src/components/ui/Slider.vue`
- Create: `app/src/components/ui/Card.spec.ts`, `app/src/components/ui/Input.spec.ts`, `app/src/components/ui/Checkbox.spec.ts`, `app/src/components/ui/Slider.spec.ts`
- Create: `app/src/pages/SettingsPage.spec.ts`
- Modify: `app/src/pages/SettingsPage.vue`

**Interfaces:**
- Consumes: `cn` from `@/lib/utils` (Task 1); `useAppStore` from `@/store/appStore` with getters `getFactorioArchivePath`, `getWorkspacePath`, `getClientCount`, `getRecreateLevel`, `getEnableAutostart`, `getRestapiPort`, `getMapExchangeString`, `getSeed`, `getSettings` and actions `updateFactorioArchivePath(string)`, `updateWorkspacePath(string)`, `updateClientCount(number)`, `updateRecreateLevel(boolean)`, `updateEnableAutostart(boolean)`, `updateRestapiPort(number)`, `updateMapExchangeString(string)`, `updateSeed(string)`, `fileExists(path): Promise<boolean>`.
- Produces:
  - `Card.vue` — props `{title?: string}`, slots `title` and default.
  - `Label.vue` — props `{for?: string}`, default slot.
  - `Input.vue` — `defineModel<string>({required: true})`, props `{invalid?: boolean; class?: HTMLAttributes['class']}`, attribute passthrough (`type`, `min`, `max`, `id`, `placeholder`).
  - `Checkbox.vue` — `defineModel<boolean>({required: true})`, props `{label: string}`.
  - `Slider.vue` — `defineModel<number>({required: true})`, props `{min?: number; max?: number; step?: number; label?: string}`.

- [ ] **Step 1: Write the four failing component tests**

Create `app/src/components/ui/Card.spec.ts`:

```ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Card from './Card.vue';

describe('Card', () => {
    it('renders the title prop as the card heading', () => {
        const wrapper = mount(Card, {props: {title: 'Seed'}, slots: {default: '<p>body</p>'}});
        expect(wrapper.find('h2').text()).toBe('Seed');
        expect(wrapper.text()).toContain('body');
    });

    it('renders no heading at all when there is no title', () => {
        const wrapper = mount(Card, {slots: {default: '<p>body</p>'}});
        expect(wrapper.find('h2').exists()).toBe(false);
    });

    it('lets the title slot carry markup the prop could not', () => {
        const wrapper = mount(Card, {
            props: {title: 'ignored'},
            slots: {title: '<a href="https://factorio.com/download">download</a>'}
        });
        expect(wrapper.find('h2 a').attributes('href')).toBe('https://factorio.com/download');
        expect(wrapper.text()).not.toContain('ignored');
    });
});
```

Create `app/src/components/ui/Input.spec.ts`:

```ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Input from './Input.vue';

describe('Input', () => {
    it('renders the model value into the field', () => {
        const wrapper = mount(Input, {props: {modelValue: '/tmp/workspace'}});
        expect((wrapper.element as HTMLInputElement).value).toBe('/tmp/workspace');
    });

    it('emits what the user typed', async () => {
        const wrapper = mount(Input, {props: {modelValue: ''}});
        await wrapper.setValue('abc');
        expect(wrapper.emitted('update:modelValue')).toEqual([['abc']]);
    });

    it('marks itself invalid for assistive tech and paints the danger border', () => {
        const wrapper = mount(Input, {props: {modelValue: '/nope', invalid: true}});
        expect(wrapper.attributes('aria-invalid')).toBe('true');
        expect(wrapper.classes()).toContain('border-danger');
        expect(wrapper.classes()).not.toContain('border-divider');
    });

    it('carries no aria-invalid when valid', () => {
        const wrapper = mount(Input, {props: {modelValue: '/tmp'}});
        expect(wrapper.attributes('aria-invalid')).toBeUndefined();
        expect(wrapper.classes()).toContain('border-divider');
    });

    it('passes native attributes through to the input element', () => {
        const wrapper = mount(Input, {props: {modelValue: '7492'}, attrs: {type: 'number', max: '65535'}});
        expect(wrapper.attributes('type')).toBe('number');
        expect(wrapper.attributes('max')).toBe('65535');
    });
});
```

Create `app/src/components/ui/Checkbox.spec.ts`:

```ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Checkbox from './Checkbox.vue';

describe('Checkbox', () => {
    it('reports the model value through aria-checked', () => {
        const wrapper = mount(Checkbox, {props: {modelValue: true, label: 'Recreate Level'}});
        expect(wrapper.find('[role="checkbox"]').attributes('aria-checked')).toBe('true');
    });

    it('emits the flipped value when the box is clicked', async () => {
        const wrapper = mount(Checkbox, {props: {modelValue: false, label: 'Recreate Level'}});
        await wrapper.find('[role="checkbox"]').trigger('click');
        expect(wrapper.emitted('update:modelValue')).toEqual([[true]]);
    });

    it('wires the label to the control so clicking the text toggles it', () => {
        const wrapper = mount(Checkbox, {props: {modelValue: false, label: 'Enable Autostart'}});
        const label = wrapper.find('label');
        expect(label.text()).toBe('Enable Autostart');
        expect(label.attributes('for')).toBe(wrapper.find('[role="checkbox"]').attributes('id'));
        expect(label.attributes('for')).toBeTruthy();
    });
});
```

Create `app/src/components/ui/Slider.spec.ts`:

```ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import {SliderRoot} from 'reka-ui';
import Slider from './Slider.vue';

describe('Slider', () => {
    it('hands reka-ui the single-element array it expects', () => {
        const wrapper = mount(Slider, {props: {modelValue: 4, min: 0, max: 16, label: 'Client Instances'}});
        expect(wrapper.findComponent(SliderRoot).props('modelValue')).toEqual([4]);
    });

    it('unwraps reka-ui array updates back to a number', async () => {
        const wrapper = mount(Slider, {props: {modelValue: 4, min: 0, max: 16}});
        await wrapper.findComponent(SliderRoot).vm.$emit('update:modelValue', [9]);
        // Forwarding the array unchanged would emit [[9]] here.
        expect(wrapper.emitted('update:modelValue')).toEqual([[9]]);
    });

    it('renders a focusable thumb with the slider role', () => {
        const wrapper = mount(Slider, {props: {modelValue: 4, min: 0, max: 16}});
        expect(wrapper.find('[role="slider"]').exists()).toBe(true);
    });

    it('passes the bounds through to reka-ui', () => {
        const wrapper = mount(Slider, {props: {modelValue: 4, min: 2, max: 16, step: 2}});
        expect(wrapper.findComponent(SliderRoot).props('min')).toBe(2);
        expect(wrapper.findComponent(SliderRoot).props('max')).toBe(16);
        expect(wrapper.findComponent(SliderRoot).props('step')).toBe(2);
    });
});
```

- [ ] **Step 2: Run them and watch them fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/Card.spec.ts src/components/ui/Input.spec.ts src/components/ui/Checkbox.spec.ts src/components/ui/Slider.spec.ts'
```

Expected: four suites failing with `Failed to resolve import`.

- [ ] **Step 3: Write the five components**

Create `app/src/components/ui/Card.vue`:

```vue
<script setup lang="ts">
defineProps<{title?: string}>();
</script>

<template>
  <section class="mb-4 rounded-card bg-card p-4 text-ink">
    <h2 v-if="$slots.title || title" class="mb-4 flex flex-wrap items-center gap-3 text-xl leading-tight">
      <slot name="title">{{ title }}</slot>
    </h2>
    <slot/>
  </section>
</template>
```

Create `app/src/components/ui/Label.vue`:

```vue
<script setup lang="ts">
defineProps<{for?: string}>();
</script>

<template>
  <label :for="$props.for" class="mb-1 block text-sm font-medium text-ink">
    <slot/>
  </label>
</template>
```

Create `app/src/components/ui/Input.vue`:

```vue
<script setup lang="ts">
import {computed, type HTMLAttributes} from 'vue';
import {cn} from '@/lib/utils';

const props = withDefaults(defineProps<{
  invalid?: boolean;
  class?: HTMLAttributes['class'];
}>(), {
  invalid: false,
  class: undefined
});

const model = defineModel<string>({required: true});

const classes = computed(() => cn(
  'h-9 w-full rounded-card border bg-card px-3 text-sm text-ink transition-colors placeholder:text-ink-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus disabled:opacity-60',
  props.invalid ? 'border-danger' : 'border-divider',
  props.class
));
</script>

<template>
  <input v-model="model" :class="classes" :aria-invalid="invalid || undefined">
</template>
```

Create `app/src/components/ui/Checkbox.vue`:

```vue
<script setup lang="ts">
import {useId} from 'vue';
import {CheckboxIndicator, CheckboxRoot} from 'reka-ui';
import {Check} from 'lucide-vue-next';

defineProps<{label: string}>();

const model = defineModel<boolean>({required: true});
const id = useId();
</script>

<template>
  <div class="flex items-center gap-2">
    <CheckboxRoot
      :id="id"
      v-model="model"
      class="flex size-5 cursor-pointer items-center justify-center rounded-card border border-divider bg-card text-white transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus data-[state=checked]:border-brand data-[state=checked]:bg-brand">
      <CheckboxIndicator class="flex items-center justify-center">
        <Check class="size-3.5"/>
      </CheckboxIndicator>
    </CheckboxRoot>
    <label :for="id" class="cursor-pointer text-sm text-ink">{{ label }}</label>
  </div>
</template>
```

Create `app/src/components/ui/Slider.vue`:

```vue
<script setup lang="ts">
import {computed} from 'vue';
import {SliderRange, SliderRoot, SliderThumb, SliderTrack} from 'reka-ui';

withDefaults(defineProps<{
  min?: number;
  max?: number;
  step?: number;
  label?: string;
}>(), {
  min: 0,
  max: 100,
  step: 1,
  label: undefined
});

const model = defineModel<number>({required: true});

// reka-ui models every slider as a range, so its value is always an array.
// The app has one thumb everywhere, so the array stops here.
const values = computed({
  get: () => [model.value],
  set: (next: number[]) => {
    model.value = next[0];
  }
});
</script>

<template>
  <SliderRoot
    v-model="values"
    :min="min"
    :max="max"
    :step="step"
    :aria-label="label"
    class="relative flex h-5 w-full touch-none select-none items-center">
    <SliderTrack class="relative h-1 grow rounded-card bg-divider">
      <SliderRange class="absolute h-full rounded-card bg-brand"/>
    </SliderTrack>
    <SliderThumb class="block size-4 cursor-pointer rounded-full border border-brand bg-card shadow focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"/>
  </SliderRoot>
</template>
```

- [ ] **Step 4: Run the four suites and watch them pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/Card.spec.ts src/components/ui/Input.spec.ts src/components/ui/Checkbox.spec.ts src/components/ui/Slider.spec.ts'
```

Expected: 15 passed.

- [ ] **Step 5: Write the failing Settings page test**

Create `app/src/pages/SettingsPage.spec.ts`:

```ts
// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {flushPromises, mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import * as client from '@/api/client';
import {useAppStore} from '@/store/appStore';
import type {AppSettings} from '@/models/settings';
import SettingsPage from './SettingsPage.vue';

vi.mock('@/api/client');

const settingsFixture = (): AppSettings => ({
    gui: {enable_autostart: false, enable_restapi: true},
    restapi: {port: 7492, web_root: null},
    factorio: {
        client_count: 4,
        factorio_archive_path: '/tmp/factorio.tar.xz',
        map_exchange_string: '',
        rcon_pass: 'pass',
        rcon_port: 1234,
        recreate: false,
        seed: '1234',
        workspace_path: '/tmp/workspace'
    }
});

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
    vi.mocked(client.pathExists).mockResolvedValue({exists: true});
});

async function mountPage() {
    const store = useAppStore();
    store.settings = settingsFixture();
    const wrapper = mount(SettingsPage);
    await flushPromises();
    return {store, wrapper};
}

describe('SettingsPage', () => {
    it('shows the persisted values in the fields', async () => {
        const {wrapper} = await mountPage();
        const values = wrapper.findAll('input').map(input => (input.element as HTMLInputElement).value);
        expect(values).toContain('/tmp/factorio.tar.xz');
        expect(values).toContain('/tmp/workspace');
        expect(values).toContain('1234');
    });

    it('persists a new seed through the store', async () => {
        const {store, wrapper} = await mountPage();
        const updateSeed = vi.spyOn(store, 'updateSeed').mockResolvedValue(undefined);
        await wrapper.get('[data-testid="seed-input"]').setValue('abcd');
        expect(updateSeed).toHaveBeenCalledWith('abcd');
    });

    it('persists an autostart change through the store', async () => {
        const {store, wrapper} = await mountPage();
        const updateEnableAutostart = vi.spyOn(store, 'updateEnableAutostart').mockResolvedValue(undefined);
        await wrapper.get('[data-testid="autostart-checkbox"] [role="checkbox"]').trigger('click');
        expect(updateEnableAutostart).toHaveBeenCalledWith(true);
    });

    it('flags a workspace path the server cannot see', async () => {
        vi.mocked(client.pathExists).mockResolvedValue({exists: false});
        const {wrapper} = await mountPage();
        expect(wrapper.text()).toContain('no such directory on the server');
        expect(wrapper.get('[data-testid="workspace-input"]').attributes('aria-invalid')).toBe('true');
    });

    it('shows the client count next to its slider', async () => {
        const {wrapper} = await mountPage();
        expect(wrapper.get('[data-testid="client-count-label"]').text()).toBe('Client Instances: 4');
    });

    it('carries no PrimeFlex grid classes any more', async () => {
        const {wrapper} = await mountPage();
        const html = wrapper.html();
        for (const dead of ['p-grid', 'p-col', 'p-formgrid', 'p-field', 'p-fluid', 'p-inputgroup', 'p-invalid']) {
            expect(html).not.toContain(dead);
        }
    });
});
```

- [ ] **Step 6: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/pages/SettingsPage.spec.ts'
```

Expected: FAIL — no element matches `[data-testid="seed-input"]`, and the last case fails on `p-grid`. If the last case passes before the rewrite, the page you are editing is not the page the test mounted.

- [ ] **Step 7: Rewrite the page**

Replace the whole of `app/src/pages/SettingsPage.vue` with:

```vue
<script setup lang="ts">
import {computed, ref, watch} from 'vue';
import {useAppStore} from '@/store/appStore';
import Card from '@/components/ui/Card.vue';
import Label from '@/components/ui/Label.vue';
import Input from '@/components/ui/Input.vue';
import Checkbox from '@/components/ui/Checkbox.vue';
import Slider from '@/components/ui/Slider.vue';

const appStore = useAppStore();

const factorioArchivePath = computed({
  get(): string {
    return appStore.getFactorioArchivePath as string
  },
  set(val: string) {
    appStore.updateFactorioArchivePath(val)
  }
})
const workspacePath = computed({
  get(): string {
    return appStore.getWorkspacePath as string
  },
  set(val: string) {
    appStore.updateWorkspacePath(val)
  }
})
const clientCount = computed({
  get(): number {
    return appStore.getClientCount as number
  },
  set(val: number) {
    appStore.updateClientCount(val)
  }
})
const recreateLevel = computed({
  get(): boolean {
    return appStore.getRecreateLevel === true
  },
  set(val: boolean) {
    appStore.updateRecreateLevel(val)
  }
})
const enableAutostart = computed({
  get(): boolean {
    return appStore.getEnableAutostart === true
  },
  set(val: boolean) {
    appStore.updateEnableAutostart(val)
  }
})
const restapiPort = computed({
  get(): string {
    return String(appStore.getRestapiPort ?? '')
  },
  set(val: string) {
    appStore.updateRestapiPort(parseInt(val, 10))
  }
})
const mapExchangeString = computed({
  get(): string {
    return appStore.getMapExchangeString as string
  },
  set(val: string) {
    appStore.updateMapExchangeString(val)
  }
})
const seed = computed({
  get(): string {
    return appStore.getSeed as string
  },
  set(val: string) {
    appStore.updateSeed(val)
  }
})

// These are the *server's* paths, not the viewer's, so the only way to tell
// the user whether they are real is to ask the server.
const isWorkspacePathValid = ref(true)
const isFactorioArchivePathValid = ref(true)

async function revalidate() {
  if (!appStore.settings) {
    return
  }
  isWorkspacePathValid.value = await appStore.fileExists(appStore.settings.factorio.workspace_path)
  isFactorioArchivePathValid.value = await appStore.fileExists(appStore.settings.factorio.factorio_archive_path)
}

watch(() => appStore.getWorkspacePath, revalidate)
watch(() => appStore.getFactorioArchivePath, revalidate)
revalidate()

const settings = computed(() => appStore.getSettings)
</script>

<template>
  <div v-if="settings" class="mx-auto max-w-3xl">
    <Card title="Settings">
      <p class="text-ink-muted">Everything below is stored on the server and applies to the Factorio instances it manages.</p>
    </Card>

    <Card>
      <template #title>
        Factorio Archive &mdash; download from
        <a class="text-link" href="https://factorio.com/download" target="_blank" rel="noopener noreferrer">factorio.com/download</a>
      </template>
      <Label for="factorio-archive-path">Archive path on the server</Label>
      <Input
        id="factorio-archive-path"
        v-model="factorioArchivePath"
        :invalid="!isFactorioArchivePathValid"
        data-testid="archive-input"/>
      <p v-if="!isFactorioArchivePathValid" class="mt-1 text-sm text-danger">no such file on the server</p>
    </Card>

    <Card title="Workspace Folder">
      <Label for="workspace-path">Directory path on the server</Label>
      <Input
        id="workspace-path"
        v-model="workspacePath"
        :invalid="!isWorkspacePathValid"
        data-testid="workspace-input"/>
      <p v-if="!isWorkspacePathValid" class="mt-1 text-sm text-danger">no such directory on the server</p>
    </Card>

    <Card title="Startup">
      <div class="flex flex-col gap-3 sm:flex-row sm:gap-8">
        <Checkbox v-model="recreateLevel" label="Recreate level on start" data-testid="recreate-checkbox"/>
        <Checkbox v-model="enableAutostart" label="Start Factorio automatically" data-testid="autostart-checkbox"/>
      </div>
    </Card>

    <Card title="HTTP API">
      <p class="mb-3 text-ink-muted">
        This page is served by the same server that exposes the API.
        <a class="text-link" href="/swagger-ui/" target="_blank" rel="noopener noreferrer">Swagger UI</a>
        &middot;
        <a class="text-link" href="/openapi.json" target="_blank" rel="noopener noreferrer">openapi.json</a>
      </p>
      <Label for="restapi-port">Port (applies on the next server start)</Label>
      <Input
        id="restapi-port"
        v-model="restapiPort"
        type="number"
        min="1"
        max="65535"
        data-testid="port-input"/>
    </Card>

    <Card title="Map Exchange String">
      <Label for="map-exchange-string">Pasted from Factorio's map generator</Label>
      <Input id="map-exchange-string" v-model="mapExchangeString" data-testid="map-exchange-input"/>
    </Card>

    <Card title="Seed">
      <Label for="seed">Map seed</Label>
      <Input id="seed" v-model="seed" data-testid="seed-input"/>
    </Card>

    <Card title="Factorio Client Instances">
      <Label data-testid="client-count-label">Client Instances: {{ clientCount }}</Label>
      <Slider v-model="clientCount" :min="0" :max="16" label="Client Instances"/>
    </Card>
  </div>
</template>
```

Note the two behaviour-preserving simplifications: the two path checks share one `revalidate()` instead of two near-identical async functions, and `restapiPort`'s getter now stringifies instead of casting `number` through `as any`.

- [ ] **Step 8: Run the page test and watch it pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/pages/SettingsPage.spec.ts'
```

Expected: 6 passed.

- [ ] **Step 9: Verify and commit**

```bash
grep -rn "primevue/inputtext\|primevue/checkbox\|primevue/slider" app/src
```

Expected: no output.

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint && pnpm run test:coverage && pnpm run build:web'
```

```bash
git commit --only app/src/components/ui/Card.vue app/src/components/ui/Card.spec.ts app/src/components/ui/Label.vue app/src/components/ui/Input.vue app/src/components/ui/Input.spec.ts app/src/components/ui/Checkbox.vue app/src/components/ui/Checkbox.spec.ts app/src/components/ui/Slider.vue app/src/components/ui/Slider.spec.ts app/src/pages/SettingsPage.vue app/src/pages/SettingsPage.spec.ts -m "feat(app): rebuild the settings page on tailwind form controls"
```

---

## Task 7: Textarea and the RCON page

**Files:**
- Create: `app/src/components/ui/Textarea.vue`, `app/src/components/ui/Textarea.spec.ts`
- Create: `app/src/pages/RconPage.spec.ts`
- Modify: `app/src/pages/RconPage.vue`

**Interfaces:**
- Consumes: `cn` (Task 1), `Button` (Task 4), `Card` (Task 6), `useToast` (Task 3); `useRconStore` from `@/store/rconStore` with getter `isExecuting` and action `execute(command: string): Promise<void>` (plan 5's Task 8 made it reject instead of swallowing).
- Produces: `@/components/ui/Textarea.vue` — `defineModel<string>({required: true})`, props `{class?: HTMLAttributes['class']}`.

- [ ] **Step 1: Write the failing Textarea test**

Create `app/src/components/ui/Textarea.spec.ts`:

```ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import Textarea from './Textarea.vue';

describe('Textarea', () => {
    it('renders the model value', () => {
        const wrapper = mount(Textarea, {props: {modelValue: '/server-save'}});
        expect(wrapper.element.tagName).toBe('TEXTAREA');
        expect((wrapper.element as HTMLTextAreaElement).value).toBe('/server-save');
    });

    it('emits what the user typed', async () => {
        const wrapper = mount(Textarea, {props: {modelValue: ''}});
        await wrapper.setValue('/c game.print(1)');
        expect(wrapper.emitted('update:modelValue')).toEqual([['/c game.print(1)']]);
    });

    it('merges a caller height over its own', () => {
        const wrapper = mount(Textarea, {props: {modelValue: '', class: 'min-h-64'}});
        expect(wrapper.classes()).toContain('min-h-64');
        expect(wrapper.classes()).not.toContain('min-h-24');
    });
});
```

- [ ] **Step 2: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/Textarea.spec.ts'
```

Expected: FAIL — `Failed to resolve import "./Textarea.vue"`.

- [ ] **Step 3: Write the component**

Create `app/src/components/ui/Textarea.vue`:

```vue
<script setup lang="ts">
import {computed, type HTMLAttributes} from 'vue';
import {cn} from '@/lib/utils';

const props = withDefaults(defineProps<{
  class?: HTMLAttributes['class'];
}>(), {
  class: undefined
});

const model = defineModel<string>({required: true});

const classes = computed(() => cn(
  'min-h-24 w-full rounded-card border border-divider bg-card px-3 py-2 font-mono text-sm text-ink transition-colors placeholder:text-ink-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus',
  props.class
));
</script>

<template>
  <textarea v-model="model" :class="classes"></textarea>
</template>
```

- [ ] **Step 4: Run it and watch it pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/Textarea.spec.ts'
```

Expected: 3 passed.

- [ ] **Step 5: Write the failing page test**

Create `app/src/pages/RconPage.spec.ts`:

```ts
// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
import {flushPromises, mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import {useRconStore} from '@/store/rconStore';
import {toastMessages, useToast} from '@/composables/useToast';
import RconPage from './RconPage.vue';

vi.mock('@/api/client');

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
    useToast().clear();
});

afterEach(() => useToast().clear());

describe('RconPage', () => {
    it('sends the typed command when Run is pressed', async () => {
        const store = useRconStore();
        const execute = vi.spyOn(store, 'execute').mockResolvedValue(undefined);
        const wrapper = mount(RconPage);

        await wrapper.get('textarea').setValue('/server-save');
        await wrapper.get('[data-testid="run-button"]').trigger('click');

        expect(execute).toHaveBeenCalledWith('/server-save');
    });

    it('sends the cheat command wired to its own button', async () => {
        const store = useRconStore();
        const execute = vi.spyOn(store, 'execute').mockResolvedValue(undefined);
        const wrapper = mount(RconPage);

        await wrapper.get('[data-testid="cheat-furnaces"]').trigger('click');

        expect(execute).toHaveBeenCalledWith(
            '/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'stone-furnace\', 20)'
        );
    });

    it('reports a failed command as an error toast instead of silently', async () => {
        const store = useRconStore();
        vi.spyOn(store, 'execute').mockRejectedValue(new Error('no Factorio instance is running'));
        const wrapper = mount(RconPage);

        await wrapper.get('[data-testid="run-button"]').trigger('click');
        await flushPromises();

        expect(toastMessages.length).toBe(1);
        expect(toastMessages[0].severity).toBe('error');
        expect(toastMessages[0].detail).toBe('no Factorio instance is running');
    });

    it('disables Run and renames it while a command is in flight', async () => {
        const store = useRconStore();
        store.executing = true;
        const wrapper = mount(RconPage);

        const run = wrapper.get('[data-testid="run-button"]');
        expect(run.attributes('disabled')).toBeDefined();
        expect(run.text()).toBe('Running ...');
    });

    it('carries no PrimeFlex grid classes any more', () => {
        const wrapper = mount(RconPage);
        expect(wrapper.html()).not.toContain('p-grid');
        expect(wrapper.html()).not.toContain('p-col');
    });
});
```

- [ ] **Step 6: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/pages/RconPage.spec.ts'
```

Expected: FAIL — no `[data-testid="run-button"]`, and the last case fails on `p-grid`.

- [ ] **Step 7: Rewrite the page**

Replace the whole of `app/src/pages/RconPage.vue` with:

```vue
<script setup lang="ts">
import {computed, ref} from 'vue';
import {useRconStore} from '@/store/rconStore';
import {useToast} from '@/composables/useToast';
import Card from '@/components/ui/Card.vue';
import Button from '@/components/ui/Button.vue';
import Textarea from '@/components/ui/Textarea.vue';

const rconStore = useRconStore()
const toast = useToast()

const command = ref('')
const isExecuting = computed(() => rconStore.isExecuting)

const execute = async (command: string) => {
  try {
    await rconStore.execute(command)
  } catch (err) {
    if (err instanceof Error) {
      toast.add({severity: 'error', summary: 'Failed to execute rcon', detail: err.message, life: 10000})
    }
  }
}

const cheats = [
  {
    testid: 'cheat-furnaces',
    label: 'Cheat Furnaces',
    command: '/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'stone-furnace\', 20)'
  },
  {
    testid: 'cheat-belts',
    label: 'Cheat Belts',
    command: '/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'transport-belt\', 100)'
  },
  {
    testid: 'cheat-drills',
    label: 'Cheat Drills',
    command: '/silent-command remote.call(\'botbridge\', \'cheat_item\', 1, \'burner-mining-drill\', 20)'
  },
  {
    testid: 'server-save',
    label: 'Save',
    command: '/server-save'
  }
]
</script>

<template>
  <div class="mx-auto max-w-3xl">
    <Card>
      <template #title>
        <span class="grow">RCON</span>
        <Button :disabled="isExecuting" data-testid="run-button" @click="execute(command)">
          {{ isExecuting ? 'Running ...' : 'Run' }}
        </Button>
      </template>

      <Textarea v-model="command" class="min-h-32" placeholder="/silent-command game.print('hello')"/>

      <div class="mt-4 flex flex-wrap gap-2">
        <Button
          v-for="cheat in cheats"
          :key="cheat.testid"
          variant="ghost"
          :data-testid="cheat.testid"
          @click="execute(cheat.command)">
          {{ cheat.label }}
        </Button>
      </div>
    </Card>
  </div>
</template>
```

- [ ] **Step 8: Run the page test and watch it pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/pages/RconPage.spec.ts'
```

Expected: 5 passed.

- [ ] **Step 9: Verify and commit**

```bash
grep -rn "primevue/textarea" app/src
```

Expected: no output.

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint && pnpm run test:coverage'
```

```bash
git commit --only app/src/components/ui/Textarea.vue app/src/components/ui/Textarea.spec.ts app/src/pages/RconPage.vue app/src/pages/RconPage.spec.ts -m "feat(app): rebuild the rcon page on the tailwind textarea and button"
```

---

## Task 8: Splitter and the Script page layout

**Files:**
- Modify: `app/src/pages/ScriptPage.vue`
- Create: `app/src/pages/ScriptPage.spec.ts`

**Interfaces:**
- Consumes: `Card` (Task 6), `Button` (Task 4), `useToast` (Task 3); reka-ui's `SplitterGroup`, `SplitterPanel`, `SplitterResizeHandle`; `useScriptStore` from `@/store/scriptStore` with getters `getCode`, `getLanguage`, `getActiveScriptPath`, `getStdout`, `getStderr`, `isExecuting` and actions `setCode(code)`, `executeScript()`, `loadScriptFile(path)`, `stopWatching()`.
- Produces: `ScriptPage.vue` with `data-testid` hooks `script-tree-pane`, `editor-pane`, `output-pane`, `run-button`. `ScriptTree.vue` is untouched here — Task 9 rewrites it, and this task's test stubs it.

reka-ui's `SplitterGroup` requires `direction` and takes `auto-save-id` to persist panel sizes to `localStorage`, which is what PrimeVue's `stateKey` + `stateStorage="local"` did. Panels must be separated by a `SplitterResizeHandle`; PrimeVue drew the gutter implicitly.

- [ ] **Step 1: Write the failing test**

Create `app/src/pages/ScriptPage.spec.ts`:

```ts
// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import {SplitterGroup} from 'reka-ui';
import {useScriptStore} from '@/store/scriptStore';
import ScriptPage from './ScriptPage.vue';

vi.mock('@/api/client');
vi.mock('@/api/jobEvents');

// Monaco needs a real canvas and web workers, and ScriptTree talks to the
// store on mount; neither is what this page's own layout test is about.
const stubs = {Editor: true, ScriptTree: true};

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
});

describe('ScriptPage', () => {
    it('shows only the tree until a script is opened', () => {
        const wrapper = mount(ScriptPage, {global: {stubs}});
        expect(wrapper.find('[data-testid="script-tree-pane"]').exists()).toBe(true);
        expect(wrapper.find('[data-testid="editor-pane"]').exists()).toBe(false);
    });

    it('shows the editor and output panes once a script is active', () => {
        const store = useScriptStore();
        store.activeScriptPath = '/example.lua';
        const wrapper = mount(ScriptPage, {global: {stubs}});
        expect(wrapper.find('[data-testid="editor-pane"]').exists()).toBe(true);
        expect(wrapper.find('[data-testid="output-pane"]').exists()).toBe(true);
    });

    it('persists the pane layout under a stable id', () => {
        const wrapper = mount(ScriptPage, {global: {stubs}});
        expect(wrapper.findComponent(SplitterGroup).props('autoSaveId')).toBe('luaScriptSplitter');
        expect(wrapper.findComponent(SplitterGroup).props('direction')).toBe('horizontal');
    });

    it('runs the active script when Run is pressed', async () => {
        const store = useScriptStore();
        store.activeScriptPath = '/example.lua';
        const executeScript = vi.spyOn(store, 'executeScript').mockResolvedValue(undefined);
        const wrapper = mount(ScriptPage, {global: {stubs}});

        await wrapper.get('[data-testid="run-button"]').trigger('click');

        expect(executeScript).toHaveBeenCalledTimes(1);
    });

    it('renders stderr lines separately from stdout lines', () => {
        const store = useScriptStore();
        store.activeScriptPath = '/example.lua';
        store.stdout = 'hello\nworld';
        store.stderr = 'careful';
        const wrapper = mount(ScriptPage, {global: {stubs}});

        expect(wrapper.findAll('[data-testid="stdout-line"]').length).toBe(2);
        expect(wrapper.findAll('[data-testid="stderr-line"]').length).toBe(1);
        expect(wrapper.get('[data-testid="stderr-line"]').text()).toBe('careful');
    });

    it('carries no PrimeFlex grid classes any more', () => {
        const wrapper = mount(ScriptPage, {global: {stubs}});
        expect(wrapper.html()).not.toContain('p-grid');
        expect(wrapper.html()).not.toContain('p-col');
    });
});
```

- [ ] **Step 2: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/pages/ScriptPage.spec.ts'
```

Expected: FAIL — `findComponent(SplitterGroup)` finds nothing and no `data-testid` matches, plus the PrimeFlex case fails on `p-grid`.

- [ ] **Step 3: Rewrite the page**

Replace the whole of `app/src/pages/ScriptPage.vue` with:

```vue
<script setup lang="ts">
import {computed, onUnmounted} from 'vue';
import ansiHTML from 'ansi-html';
import {SplitterGroup, SplitterPanel, SplitterResizeHandle} from 'reka-ui';
import {useDebounceFn} from '@vueuse/core';
import {useScriptStore} from '@/store/scriptStore';
import {useToast} from '@/composables/useToast';
import ScriptTree from '@/components/ScriptTree.vue';
import Editor from '@/components/Editor.vue';
import Card from '@/components/ui/Card.vue';
import Button from '@/components/ui/Button.vue';

const scriptStore = useScriptStore()
const toast = useToast()

const code = computed(() => scriptStore.getCode)
const language = computed(() => scriptStore.getLanguage)
const activeScriptPath = computed(() => scriptStore.getActiveScriptPath)
const stdout = computed(() => scriptStore.getStdout)
const stderr = computed(() => scriptStore.getStderr)
const isExecuting = computed(() => scriptStore.isExecuting)

const updateCode = useDebounceFn((code: string) => {
  scriptStore.setCode(code)
}, 1000)

const execute = async () => {
  try {
    await scriptStore.executeScript()
  } catch (err) {
    if (err instanceof Error) {
      toast.add({severity: 'error', summary: 'Failed to execute script', detail: err.message, life: 10000})
    }
  }
}

const loadScriptFile = (path: string) => scriptStore.loadScriptFile(path)

// The SSE connection outlives the component otherwise: navigating away from
// the page would leave an open stream appending into a store nothing renders.
onUnmounted(() => {
  scriptStore.stopWatching()
})
</script>

<template>
  <Card>
    <template #title>
      <span class="grow">Lua Script <strong class="font-mono text-base">{{ activeScriptPath }}</strong></span>
      <Button :disabled="isExecuting" data-testid="run-button" @click="execute()">
        {{ isExecuting ? 'Running ...' : 'Run' }}
      </Button>
    </template>

    <SplitterGroup direction="horizontal" auto-save-id="luaScriptSplitter" class="h-[70vh] w-full">
      <SplitterPanel :default-size="20" :min-size="10" class="overflow-auto pr-2" data-testid="script-tree-pane">
        <ScriptTree @select="loadScriptFile($event)"/>
      </SplitterPanel>

      <SplitterResizeHandle class="w-1 rounded-card bg-divider transition-colors hover:bg-brand"/>

      <SplitterPanel v-if="activeScriptPath" :default-size="80" class="pl-2">
        <SplitterGroup direction="vertical" auto-save-id="luaScriptOutputSplitter" class="h-full">
          <SplitterPanel :default-size="70" class="overflow-hidden" data-testid="editor-pane">
            <Editor class="size-full" :value="code" :language="language" theme="vs-dark" @change="updateCode"/>
          </SplitterPanel>

          <SplitterResizeHandle class="h-1 rounded-card bg-divider transition-colors hover:bg-brand"/>

          <SplitterPanel :default-size="30" class="overflow-auto bg-card font-mono text-xs" data-testid="output-pane">
            <pre
              v-for="(line, idx) in stderr.split('\n')"
              :key="'stderr' + idx"
              class="m-0 whitespace-pre-wrap text-danger"
              data-testid="stderr-line"
              v-html="ansiHTML(line)"></pre>
            <pre
              v-for="(line, idx) in stdout.split('\n')"
              :key="'stdout' + idx"
              class="m-0 whitespace-pre-wrap text-ink"
              data-testid="stdout-line"
              v-html="ansiHTML(line)"></pre>
          </SplitterPanel>
        </SplitterGroup>
      </SplitterPanel>
    </SplitterGroup>
  </Card>
</template>
```

Two notes for the implementer. The `:innerHTML` bindings became `v-html`, which is the same thing spelled the way `eslint-plugin-vue` expects; the content is ANSI-escaped process output rendered by `ansi-html`, exactly as before. And an empty `stderr` splits to `['']`, so one empty `<pre>` renders — that was true before this change too, and the test asserts on a non-empty value.

- [ ] **Step 4: Run the test and watch it pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/pages/ScriptPage.spec.ts'
```

Expected: 6 passed. If the "shows only the tree" case fails because `stderr.split` runs on an undefined store field, the store fixture is wrong, not the page.

- [ ] **Step 5: Verify and commit**

```bash
grep -rn "primevue/splitter" app/src
```

Expected: no output.

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint && pnpm run test:coverage && pnpm run build:web'
```

```bash
git commit --only app/src/pages/ScriptPage.vue app/src/pages/ScriptPage.spec.ts -m "feat(app): move the script page onto the reka-ui splitter"
```

---

## Task 9: The tree — the one real feature rebuild

PrimeVue's `Tree` is the only component with no equivalent worth adopting (see "Component decisions" above). Its lazy-expand contract has to be reimplemented: `ScriptTree` lists `/` on mount, and each directory's children are fetched on first expansion and spliced into the tree by key.

**Files:**
- Create: `app/src/components/ui/tree/mergeNodes.ts`, `app/src/components/ui/tree/mergeNodes.spec.ts`
- Create: `app/src/components/ui/tree/TreeView.vue`, `app/src/components/ui/tree/TreeView.spec.ts`
- Modify: `app/src/components/ScriptTree.vue`
- Create: `app/src/components/ScriptTree.spec.ts`

**Interfaces:**
- Consumes: `ScriptTreeNode` from `@/api/types` — `{key: string; label: string; leaf: boolean; children: ScriptTreeNode[]}` (renamed from `PrimeVueTreeNode` by plan 5's Task 2; plan 5's Task 13 then deleted the TypeScript generator and `app/src/models/types.ts` with it, so this type is now hand-written in `app/src/api/types.ts` and checked against the OpenAPI spec by the contract test). `useScriptStore` from `@/store/scriptStore` with `loadScriptsInDirectory(path: string): Promise<ScriptTreeNode[]>` and getter `getLoadingScriptsInDirectory: boolean`.
- Produces:
  - `mergeNodes(nodes: ScriptTreeNode[], key: string, children: ScriptTreeNode[]): ScriptTreeNode[]` from `@/components/ui/tree/mergeNodes` — pure, returns a new array, replaces the children of the node whose `key` matches at any depth.
  - `@/components/ui/tree/TreeView.vue` — props `{nodes: ScriptTreeNode[]; expandedKeys: string[]; selectedKey: string | null; level?: number}`, emits `toggle(node: ScriptTreeNode)` and `select(node: ScriptTreeNode)`. Renders `<ul role="tree">` at level 0 and `<ul role="group">` below, one `<li role="treeitem">` per node.
  - `ScriptTree.vue` keeps its existing public contract: it emits `select` with the node **key** (a string), which `ScriptPage.vue` passes to `scriptStore.loadScriptFile`.

- [ ] **Step 1: Write the failing merge test**

Create `app/src/components/ui/tree/mergeNodes.spec.ts`:

```ts
import {describe, expect, it} from 'vitest';
import type {ScriptTreeNode} from '@/api/types';
import {mergeNodes} from './mergeNodes';

const leaf = (key: string, label: string): ScriptTreeNode => ({key, label, leaf: true, children: []});
const dir = (key: string, label: string, children: ScriptTreeNode[] = []): ScriptTreeNode =>
    ({key, label, leaf: false, children});

describe('mergeNodes', () => {
    it('fills in the children of a top-level directory', () => {
        const tree = [dir('/sub', 'sub'), leaf('/a.lua', 'a.lua')];
        const merged = mergeNodes(tree, '/sub', [leaf('/sub/b.lua', 'b.lua')]);
        expect(merged[0].children.map(node => node.key)).toEqual(['/sub/b.lua']);
    });

    it('fills in the children of a nested directory without disturbing its siblings', () => {
        const tree = [
            dir('/sub', 'sub', [dir('/sub/deep', 'deep'), leaf('/sub/b.lua', 'b.lua')]),
            leaf('/a.lua', 'a.lua')
        ];
        const merged = mergeNodes(tree, '/sub/deep', [leaf('/sub/deep/c.lua', 'c.lua')]);

        // A top-level-only implementation leaves this empty.
        expect(merged[0].children[0].children.map(node => node.key)).toEqual(['/sub/deep/c.lua']);
        expect(merged[0].children[1].key).toBe('/sub/b.lua');
        expect(merged[1].key).toBe('/a.lua');
    });

    it('leaves the tree unchanged when the key is not in it', () => {
        const tree = [dir('/sub', 'sub')];
        const merged = mergeNodes(tree, '/nope', [leaf('/nope/x.lua', 'x.lua')]);
        expect(merged[0].children).toEqual([]);
    });

    it('does not mutate the input', () => {
        const tree = [dir('/sub', 'sub')];
        mergeNodes(tree, '/sub', [leaf('/sub/b.lua', 'b.lua')]);
        expect(tree[0].children).toEqual([]);
    });
});
```

- [ ] **Step 2: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/tree/mergeNodes.spec.ts'
```

Expected: FAIL — `Failed to resolve import "./mergeNodes"`.

- [ ] **Step 3: Write the merge**

Create `app/src/components/ui/tree/mergeNodes.ts`:

```ts
import type {ScriptTreeNode} from '@/api/types';

/**
 * Return a copy of `nodes` in which the node identified by `key` has `children`.
 *
 * Directory listings arrive one directory at a time (`GET /api/v1/scripts`),
 * so an expanded directory's children have to be spliced into a tree that is
 * already on screen. Matching on `key` -- the node's full path -- rather than
 * walking the path's segments by label means a directory whose name repeats at
 * another depth cannot be confused for its namesake.
 *
 * Pure on purpose: the caller assigns the result, so Vue sees one replacement
 * rather than a mutation it has to detect.
 */
export function mergeNodes(
    nodes: ScriptTreeNode[],
    key: string,
    children: ScriptTreeNode[]
): ScriptTreeNode[] {
    return nodes.map(node => {
        if (node.key === key) {
            return {...node, children};
        }
        if (node.children.length === 0) {
            return node;
        }
        return {...node, children: mergeNodes(node.children, key, children)};
    });
}
```

- [ ] **Step 4: Run it and watch it pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/tree/mergeNodes.spec.ts'
```

Expected: 4 passed.

- [ ] **Step 5: Write the failing TreeView test**

Create `app/src/components/ui/tree/TreeView.spec.ts`:

```ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import type {ScriptTreeNode} from '@/api/types';
import TreeView from './TreeView.vue';

const leaf = (key: string, label: string): ScriptTreeNode => ({key, label, leaf: true, children: []});
const dir = (key: string, label: string, children: ScriptTreeNode[] = []): ScriptTreeNode =>
    ({key, label, leaf: false, children});

const nodes = [dir('/sub', 'sub', [leaf('/sub/b.lua', 'b.lua')]), leaf('/a.lua', 'a.lua')];

function mountTree(expandedKeys: string[] = [], selectedKey: string | null = null) {
    return mount(TreeView, {props: {nodes, expandedKeys, selectedKey}});
}

describe('TreeView', () => {
    it('renders one row per node and marks the root list as a tree', () => {
        const wrapper = mountTree();
        expect(wrapper.element.getAttribute('role')).toBe('tree');
        expect(wrapper.findAll('[role="treeitem"]').length).toBe(2);
        expect(wrapper.findAll('button')[1].text()).toContain('a.lua');
    });

    it('hides the children of a collapsed directory', () => {
        const wrapper = mountTree();
        expect(wrapper.find('[role="group"]').exists()).toBe(false);
        expect(wrapper.text()).not.toContain('b.lua');
    });

    it('renders the children of an expanded directory in a nested group', () => {
        const wrapper = mountTree(['/sub']);
        expect(wrapper.find('[role="group"]').exists()).toBe(true);
        expect(wrapper.text()).toContain('b.lua');
    });

    it('reports expansion state on the directory row only', () => {
        const wrapper = mountTree(['/sub']);
        const rows = wrapper.findAll('[role="treeitem"]');
        expect(rows[0].attributes('aria-expanded')).toBe('true');
        expect(rows[1].attributes('aria-expanded')).toBeUndefined();
    });

    it('emits toggle for a directory and select for a file', async () => {
        const wrapper = mountTree();
        const buttons = wrapper.findAll('button');

        await buttons[0].trigger('click');
        await buttons[1].trigger('click');

        expect(wrapper.emitted('toggle')?.[0][0]).toMatchObject({key: '/sub'});
        expect(wrapper.emitted('select')?.[0][0]).toMatchObject({key: '/a.lua'});
        expect(wrapper.emitted('select')?.length).toBe(1);
    });

    it('marks the selected file and only that file', () => {
        const wrapper = mountTree([], '/a.lua');
        const rows = wrapper.findAll('[role="treeitem"]');
        expect(rows[1].attributes('aria-selected')).toBe('true');
        expect(rows[0].attributes('aria-selected')).toBe('false');
    });

    it('bubbles a nested selection up through the recursion', async () => {
        const wrapper = mountTree(['/sub']);
        const nested = wrapper.find('[role="group"] button');
        await nested.trigger('click');
        expect(wrapper.emitted('select')?.[0][0]).toMatchObject({key: '/sub/b.lua'});
    });
});
```

- [ ] **Step 6: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/tree/TreeView.spec.ts'
```

Expected: FAIL — `Failed to resolve import "./TreeView.vue"`.

- [ ] **Step 7: Write the TreeView**

Create `app/src/components/ui/tree/TreeView.vue`:

```vue
<script setup lang="ts">
import {ChevronDown, ChevronRight, FileCode} from 'lucide-vue-next';
import type {ScriptTreeNode} from '@/api/types';

// Self-referencing by filename: Vue resolves <TreeView> inside this template
// to this component, which is how the recursion works without a named export.
const props = withDefaults(defineProps<{
  nodes: ScriptTreeNode[];
  expandedKeys: string[];
  selectedKey: string | null;
  level?: number;
}>(), {
  level: 0
});

const emit = defineEmits<{
  toggle: [node: ScriptTreeNode];
  select: [node: ScriptTreeNode];
}>();

function isExpanded(node: ScriptTreeNode): boolean {
  return props.expandedKeys.includes(node.key);
}

function activate(node: ScriptTreeNode): void {
  if (node.leaf) {
    emit('select', node);
  } else {
    emit('toggle', node);
  }
}
</script>

<template>
  <ul :role="level === 0 ? 'tree' : 'group'" class="m-0 list-none p-0 text-sm">
    <li
      v-for="node in nodes"
      :key="node.key"
      role="treeitem"
      :aria-expanded="node.leaf ? undefined : isExpanded(node)"
      :aria-selected="selectedKey === node.key">
      <button
        type="button"
        class="flex w-full cursor-pointer items-center gap-1 rounded-card px-2 py-1 text-left transition-colors hover:bg-divider focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
        :class="selectedKey === node.key ? 'bg-divider font-medium text-brand' : 'text-ink'"
        :style="{paddingLeft: (level * 12 + 8) + 'px'}"
        @click="activate(node)">
        <ChevronDown v-if="!node.leaf && isExpanded(node)" class="size-4 shrink-0" aria-hidden="true"/>
        <ChevronRight v-else-if="!node.leaf" class="size-4 shrink-0" aria-hidden="true"/>
        <FileCode v-else class="size-4 shrink-0 text-ink-muted" aria-hidden="true"/>
        <span class="truncate">{{ node.label }}</span>
      </button>

      <TreeView
        v-if="!node.leaf && isExpanded(node)"
        :nodes="node.children"
        :expanded-keys="expandedKeys"
        :selected-key="selectedKey"
        :level="level + 1"
        @toggle="emit('toggle', $event)"
        @select="emit('select', $event)"/>
    </li>
  </ul>
</template>
```

- [ ] **Step 8: Run it and watch it pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ui/tree/TreeView.spec.ts'
```

Expected: 7 passed.

- [ ] **Step 9: Write the failing ScriptTree test**

Create `app/src/components/ScriptTree.spec.ts`:

```ts
// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {flushPromises, mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import * as client from '@/api/client';
import type {ScriptTreeNode} from '@/api/types';
import ScriptTree from './ScriptTree.vue';

vi.mock('@/api/client');

const leaf = (key: string, label: string): ScriptTreeNode => ({key, label, leaf: true, children: []});
const dir = (key: string, label: string): ScriptTreeNode => ({key, label, leaf: false, children: []});

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
});

async function mountTree() {
    const wrapper = mount(ScriptTree);
    await flushPromises();
    return wrapper;
}

describe('ScriptTree', () => {
    it('lists the workspace root on mount', async () => {
        vi.mocked(client.listScripts).mockResolvedValue([dir('/sub', 'sub'), leaf('/a.lua', 'a.lua')]);
        const wrapper = await mountTree();

        expect(client.listScripts).toHaveBeenCalledWith('/');
        expect(wrapper.findAll('[role="treeitem"]').length).toBe(2);
    });

    it('fetches a directory the first time it is expanded and shows its files', async () => {
        vi.mocked(client.listScripts)
            .mockResolvedValueOnce([dir('/sub', 'sub')])
            .mockResolvedValueOnce([leaf('/sub/b.lua', 'b.lua')]);
        const wrapper = await mountTree();

        await wrapper.get('button').trigger('click');
        await flushPromises();

        expect(client.listScripts).toHaveBeenLastCalledWith('/sub');
        expect(wrapper.text()).toContain('b.lua');
    });

    it('does not re-fetch a directory that is collapsed and expanded again', async () => {
        vi.mocked(client.listScripts)
            .mockResolvedValueOnce([dir('/sub', 'sub')])
            .mockResolvedValueOnce([leaf('/sub/b.lua', 'b.lua')]);
        const wrapper = await mountTree();

        await wrapper.get('button').trigger('click');
        await flushPromises();
        await wrapper.get('button').trigger('click');
        await flushPromises();
        await wrapper.get('button').trigger('click');
        await flushPromises();

        expect(vi.mocked(client.listScripts).mock.calls.map(call => call[0])).toEqual(['/', '/sub']);
        expect(wrapper.text()).toContain('b.lua');
    });

    it('emits the key of a selected file and nothing for a directory', async () => {
        vi.mocked(client.listScripts).mockResolvedValue([dir('/sub', 'sub'), leaf('/a.lua', 'a.lua')]);
        const wrapper = await mountTree();

        await wrapper.findAll('button')[1].trigger('click');
        await wrapper.findAll('button')[0].trigger('click');
        await flushPromises();

        expect(wrapper.emitted('select')).toEqual([['/a.lua']]);
    });

    it('keeps the last selected file marked', async () => {
        vi.mocked(client.listScripts).mockResolvedValue([leaf('/a.lua', 'a.lua')]);
        const wrapper = await mountTree();

        await wrapper.get('button').trigger('click');
        await flushPromises();

        expect(wrapper.get('[role="treeitem"]').attributes('aria-selected')).toBe('true');
    });

    it('reports a failed listing instead of leaving an empty tree', async () => {
        vi.mocked(client.listScripts).mockRejectedValue(new Error('scripts directory is unreadable'));
        const wrapper = await mountTree();

        expect(wrapper.text()).toContain('scripts directory is unreadable');
    });
});
```

- [ ] **Step 10: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ScriptTree.spec.ts'
```

Expected: FAIL — the old component renders PrimeVue's `Tree`, so there are no `[role="treeitem"]` elements and no error text.

- [ ] **Step 11: Rewrite `ScriptTree.vue`**

Replace the whole of `app/src/components/ScriptTree.vue` with:

```vue
<script setup lang="ts">
import {computed, onMounted, ref} from 'vue';
import {Loader2} from 'lucide-vue-next';
import type {ScriptTreeNode} from '@/api/types';
import {useScriptStore} from '@/store/scriptStore';
import TreeView from '@/components/ui/tree/TreeView.vue';
import {mergeNodes} from '@/components/ui/tree/mergeNodes';

const scriptStore = useScriptStore()

const nodes = ref([] as ScriptTreeNode[])
const expandedKeys = ref([] as string[])
const selectedKey = ref(null as string | null)
const error = ref(null as string | null)

// Directories whose listing has already been fetched. Without it, collapsing
// and re-expanding a directory would re-fetch it every time.
const loadedKeys = ref([] as string[])

const loading = computed(() => scriptStore.getLoadingScriptsInDirectory)

const emit = defineEmits<{select: [key: string]}>()

async function list(path: string): Promise<ScriptTreeNode[] | null> {
  try {
    const listed = await scriptStore.loadScriptsInDirectory(path)
    error.value = null
    return listed
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    return null
  }
}

onMounted(async () => {
  const root = await list('/')
  if (root !== null) {
    nodes.value = root
  }
})

async function onToggle(node: ScriptTreeNode) {
  if (expandedKeys.value.includes(node.key)) {
    expandedKeys.value = expandedKeys.value.filter(key => key !== node.key)
    return
  }
  if (!loadedKeys.value.includes(node.key)) {
    const children = await list(node.key)
    if (children === null) {
      return
    }
    nodes.value = mergeNodes(nodes.value, node.key, children)
    loadedKeys.value = [...loadedKeys.value, node.key]
  }
  expandedKeys.value = [...expandedKeys.value, node.key]
}

function onSelect(node: ScriptTreeNode) {
  selectedKey.value = node.key
  emit('select', node.key)
}
</script>

<template>
  <div>
    <p v-if="error" class="mb-2 text-sm text-danger" data-testid="tree-error">{{ error }}</p>
    <p v-if="loading" class="mb-2 flex items-center gap-2 text-sm text-ink-muted">
      <Loader2 class="size-4 animate-spin" aria-hidden="true"/>
      Loading ...
    </p>
    <TreeView
      :nodes="nodes"
      :expanded-keys="expandedKeys"
      :selected-key="selectedKey"
      @toggle="onToggle"
      @select="onSelect"/>
  </div>
</template>
```

- [ ] **Step 12: Run it and watch it pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/components/ScriptTree.spec.ts'
```

Expected: 6 passed.

- [ ] **Step 13: Verify and commit**

```bash
grep -rn "primevue/tree" app/src
```

Expected: no output.

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint && pnpm run test:coverage && pnpm run build:web'
```

```bash
git commit --only app/src/components/ui/tree/mergeNodes.ts app/src/components/ui/tree/mergeNodes.spec.ts app/src/components/ui/tree/TreeView.vue app/src/components/ui/tree/TreeView.spec.ts app/src/components/ScriptTree.vue app/src/components/ScriptTree.spec.ts -m "feat(app): rebuild the lazy script tree without primevue"
```

---

## Task 10: Dashboard and Tasks pages

The last two pages carrying dead PrimeFlex classes. The Dashboard is one of the two visibly broken pages: `p-col-12 p-lg-4` was meant to lay three summary cards per row and lays none.

**Files:**
- Modify: `app/src/pages/Dashboard.vue`, `app/src/pages/TasksPage.vue`
- Create: `app/src/pages/Dashboard.spec.ts`

**Interfaces:**
- Consumes: `Card` (Task 6), `useToast` (Task 3), `useAppStore` getter `getClientCount: number | null`.
- Produces: nothing other tasks consume.

- [ ] **Step 1: Write the failing test**

Create `app/src/pages/Dashboard.spec.ts`:

```ts
// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import {useAppStore} from '@/store/appStore';
import type {AppSettings} from '@/models/settings';
import Dashboard from './Dashboard.vue';

vi.mock('@/api/client');

const settingsFixture = (clientCount: number): AppSettings => ({
    gui: {enable_autostart: false, enable_restapi: true},
    restapi: {port: 7492, web_root: null},
    factorio: {
        client_count: clientCount,
        factorio_archive_path: '/tmp/factorio.tar.xz',
        map_exchange_string: '',
        rcon_pass: 'pass',
        rcon_port: 1234,
        recreate: false,
        seed: '1234',
        workspace_path: '/tmp/workspace'
    }
});

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
});

describe('Dashboard', () => {
    it('renders one client tile per configured client, plus the summary tile', () => {
        const store = useAppStore();
        store.settings = settingsFixture(3);
        const wrapper = mount(Dashboard);

        expect(wrapper.get('[data-testid="instance-count"]').text()).toBe('3');
        expect(wrapper.findAll('[data-testid="client-tile"]').length).toBe(3);
        expect(wrapper.findAll('[data-testid="client-tile"]')[2].text()).toContain('client3');
    });

    it('follows a client-count change', async () => {
        const store = useAppStore();
        store.settings = settingsFixture(1);
        const wrapper = mount(Dashboard);
        expect(wrapper.findAll('[data-testid="client-tile"]').length).toBe(1);

        store.settings = settingsFixture(4);
        await wrapper.vm.$nextTick();

        expect(wrapper.findAll('[data-testid="client-tile"]').length).toBe(4);
    });

    it('renders nothing client-shaped before settings have loaded', () => {
        const wrapper = mount(Dashboard);
        expect(wrapper.findAll('[data-testid="client-tile"]').length).toBe(0);
    });

    it('carries no PrimeFlex grid classes any more', () => {
        const store = useAppStore();
        store.settings = settingsFixture(2);
        const wrapper = mount(Dashboard);
        const html = wrapper.html();
        for (const dead of ['p-grid', 'p-col-12', 'p-lg-4', 'p-fluid']) {
            expect(html).not.toContain(dead);
        }
    });
});
```

- [ ] **Step 2: Run it and watch it fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/pages/Dashboard.spec.ts'
```

Expected: FAIL — no `[data-testid="instance-count"]`, and the last case fails on `p-grid`.

- [ ] **Step 3: Rewrite the Dashboard**

Replace the whole of `app/src/pages/Dashboard.vue` with:

```vue
<script setup lang="ts">
import {computed} from 'vue';
import {useAppStore} from '@/store/appStore';
import {useToast} from '@/composables/useToast';

const toast = useToast()
const appStore = useAppStore()

const clientCount = computed(() => appStore.getClientCount ?? 0)

// The per-client status is a placeholder until the instance API reports one
// per client; today GET /api/v1/instance answers for the group.
const clients = computed(() => Array.from({length: clientCount.value}, (_unused, index) => ({
  name: 'client' + (index + 1),
  status: 'not_initialized'
})))

const sendTestMessage = () => {
  toast.add({severity: 'info', summary: 'Info Message', detail: 'Message Content', life: 3000})
}
</script>

<template>
  <div class="grid grid-cols-1 gap-4 lg:grid-cols-3">
    <section class="relative rounded-card bg-card p-4 text-ink">
      <span class="text-xl">Instances</span>
      <span class="mt-2 block text-ink-muted">Number of configured instances</span>
      <button
        type="button"
        class="absolute right-2 top-2 cursor-pointer rounded-card bg-success px-3 py-1 text-2xl text-white"
        data-testid="instance-count"
        @click="sendTestMessage()">{{ clientCount }}</button>
    </section>

    <section
      v-for="client in clients"
      :key="client.name"
      class="rounded-card bg-card p-4 text-ink"
      data-testid="client-tile">
      <span class="text-xl">{{ client.name }}</span>
      <span class="mt-2 block text-ink-muted">{{ client.status }}</span>
    </section>
  </div>
</template>
```

This also drops the `ref` + `watch` + imperative `updateClients()` bookkeeping: the client tiles are a pure function of the client count, so a `computed` is both shorter and free of the "settings arrived after mount" hole the old code had.

- [ ] **Step 4: Rewrite the Tasks page**

Replace the whole of `app/src/pages/TasksPage.vue` with:

```vue
<script setup lang="ts">
import GanttChart from '@/components/GanttChart.vue';
import Card from '@/components/ui/Card.vue';
</script>

<template>
  <Card title="Tasks">
    <GanttChart/>
  </Card>
</template>
```

`GanttChart.vue` stays the `<div>TODO</div>` stub it is — filling it in is planner work, not redesign work.

- [ ] **Step 5: Run the tests and watch them pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/pages/Dashboard.spec.ts'
```

Expected: 4 passed.

- [ ] **Step 6: Prove no dead PrimeFlex class survives anywhere**

```bash
grep -rn "p-grid\|p-col\|p-formgrid\|p-field\|p-fluid\|p-inputgroup\|p-invalid\|p-formgroup-inline\|p-error" app/src
```

Expected: matches only in `app/src/AppConfig.vue`, which Task 11 deletes. Anything else is a file this plan missed.

- [ ] **Step 7: Verify and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint && pnpm run test:coverage'
```

```bash
git commit --only app/src/pages/Dashboard.vue app/src/pages/Dashboard.spec.ts app/src/pages/TasksPage.vue -m "feat(app): rebuild the dashboard and tasks pages on a tailwind grid"
```

---

## Task 11: The layout shell in Tailwind, and Preflight on

The hardest task and the reason the previous ten came first: nothing outside the shell depends on browser-default or SCSS bare-element styling by now, so Preflight can go on in the same commit that deletes the SCSS.

**Files:**
- Modify: `app/src/App.vue`, `app/src/AppTopbar.vue`, `app/src/AppMenu.vue`, `app/src/AppFooter.vue`, `app/src/models/dashboard.ts`, `app/src/assets/tailwind.css`
- Delete: `app/src/AppSubmenu.vue`, `app/src/AppConfig.vue`, `app/src/assets/layout/` (the whole directory: `layout.scss`, `_variables.scss`, `_overrides.scss` and `sass/_{layout,mixins,splash,main,topbar,sidebar,profile,menu,config,content,footer,responsive,utils,typography,dashboard}.scss`)
- Modify: `app/src/main.ts` (drop the `layout.scss` import)
- Create: `app/src/AppMenu.spec.ts`, `app/src/AppTopbar.spec.ts`, `app/src/App.spec.ts`

**Interfaces:**
- Consumes: `Toaster` (Task 3), `useToast` (Task 3), `useAppStore` (`getSettings`, `loadSettings`), `useInstanceStore` (`checkInstanceState`), lucide icons.
- Produces:
  - `app/src/models/dashboard.ts` exports `type MenuEntry = {label: string; icon: Component; to: string}` — replacing `DashboardMenu`, whose twelve optional fields (`items`, `command`, `url`, `badge`, `separator`, `target`, …) were never used by any menu entry.
  - `AppMenu.vue` — props `{items: MenuEntry[]}`, emits `navigate`.
  - `AppTopbar.vue` — props `{sidebarOpen: boolean}`, emits `menu-toggle`.
  - `AppFooter.vue` — no props.
  - `App.vue` — the shell: fixed topbar, fixed sidebar, `<main>`, footer, `<Toaster/>`.

- [ ] **Step 1: Write the failing shell tests**

Create `app/src/AppMenu.spec.ts`:

```ts
// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount, RouterLinkStub} from '@vue/test-utils';
import {Cog, Home, Network} from 'lucide-vue-next';
import AppMenu from './AppMenu.vue';
import type {MenuEntry} from '@/models/dashboard';

const items: MenuEntry[] = [
    {label: 'Dashboard', icon: Home, to: '/'},
    {label: 'Settings', icon: Cog, to: '/settings'},
    {label: 'Tasks', icon: Network, to: '/tasks'}
];

describe('AppMenu', () => {
    it('renders one link per entry, in order, pointing where the entry says', () => {
        const wrapper = mount(AppMenu, {
            props: {items},
            global: {stubs: {RouterLink: RouterLinkStub}}
        });

        const links = wrapper.findAllComponents(RouterLinkStub);
        expect(links.length).toBe(3);
        expect(links[2].props('to')).toBe('/tasks');
        expect(links[2].text()).toContain('Tasks');
    });

    it('renders the entry icon component, not an icon-font class string', () => {
        const wrapper = mount(AppMenu, {
            props: {items},
            global: {stubs: {RouterLink: RouterLinkStub}}
        });

        expect(wrapper.findComponent(Home).exists()).toBe(true);
        expect(wrapper.html()).not.toContain('pi-fw');
    });

    it('emits navigate when a link is clicked, so the mobile sidebar can close', async () => {
        const wrapper = mount(AppMenu, {
            props: {items},
            global: {stubs: {RouterLink: RouterLinkStub}}
        });

        await wrapper.findAllComponents(RouterLinkStub)[0].trigger('click');

        expect(wrapper.emitted('navigate')).toHaveLength(1);
    });
});
```

Create `app/src/AppTopbar.spec.ts`:

```ts
// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import {useAppStore} from '@/store/appStore';
import type {AppSettings} from '@/models/settings';
import AppTopbar from './AppTopbar.vue';

vi.mock('@/api/client');

const settingsFixture = (): AppSettings => ({
    gui: {enable_autostart: false, enable_restapi: true},
    restapi: {port: 7492, web_root: null},
    factorio: {
        client_count: 2,
        factorio_archive_path: '/tmp/factorio.tar.xz',
        map_exchange_string: '',
        rcon_pass: 'pass',
        rcon_port: 1234,
        recreate: false,
        seed: '1234',
        workspace_path: '/tmp/workspace'
    }
});

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
});

describe('AppTopbar', () => {
    it('emits menu-toggle when the hamburger is pressed', async () => {
        const wrapper = mount(AppTopbar, {props: {sidebarOpen: true}, global: {stubs: {ProcessControl: true}}});
        await wrapper.get('[data-testid="menu-toggle"]').trigger('click');
        expect(wrapper.emitted('menu-toggle')).toHaveLength(1);
    });

    it('names the toggle for assistive tech', () => {
        const wrapper = mount(AppTopbar, {props: {sidebarOpen: true}, global: {stubs: {ProcessControl: true}}});
        expect(wrapper.get('[data-testid="menu-toggle"]').attributes('aria-label')).toBe('Toggle menu');
    });

    it('shows the configured client count once settings are loaded', () => {
        const store = useAppStore();
        store.settings = settingsFixture();
        const wrapper = mount(AppTopbar, {props: {sidebarOpen: true}, global: {stubs: {ProcessControl: true}}});
        expect(wrapper.text()).toContain('2 Clients');
    });

    it('shows no client count before settings arrive', () => {
        const wrapper = mount(AppTopbar, {props: {sidebarOpen: true}, global: {stubs: {ProcessControl: true}}});
        expect(wrapper.text()).not.toContain('Clients');
    });

    it('indents itself past the sidebar only while the sidebar is open', () => {
        const open = mount(AppTopbar, {props: {sidebarOpen: true}, global: {stubs: {ProcessControl: true}}});
        const closed = mount(AppTopbar, {props: {sidebarOpen: false}, global: {stubs: {ProcessControl: true}}});
        expect(open.classes()).toContain('lg:left-sidebar');
        expect(closed.classes()).not.toContain('lg:left-sidebar');
    });
});
```

Create `app/src/App.spec.ts`:

```ts
// @vitest-environment jsdom
import {beforeEach, describe, expect, it, vi} from 'vitest';
import {flushPromises, mount} from '@vue/test-utils';
import {createPinia, setActivePinia} from 'pinia';
import * as client from '@/api/client';
import App from './App.vue';
import AppTopbar from './AppTopbar.vue';

vi.mock('@/api/client');

const settings = {
    gui: {enable_autostart: false, enable_restapi: true},
    restapi: {port: 7492, web_root: null},
    factorio: {
        client_count: 1,
        factorio_archive_path: '/tmp/factorio.tar.xz',
        map_exchange_string: '',
        rcon_pass: 'pass',
        rcon_port: 1234,
        recreate: false,
        seed: '1234',
        workspace_path: '/tmp/workspace'
    }
};

const instance = {
    started: false,
    starting: false,
    client_count: 1,
    server_port: null,
    rcon_port: null,
    last_error: null
};

beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetAllMocks();
    vi.mocked(client.getSettings).mockResolvedValue(settings);
    vi.mocked(client.getInstance).mockResolvedValue(instance);
});

async function mountApp() {
    const wrapper = mount(App, {
        global: {stubs: {RouterLink: true, RouterView: true, ProcessControl: true}}
    });
    await flushPromises();
    return wrapper;
}

describe('App shell', () => {
    it('starts with the sidebar shown on a desktop-width window', async () => {
        const wrapper = await mountApp();
        expect(wrapper.get('[data-testid="sidebar"]').classes()).toContain('translate-x-0');
    });

    it('slides the sidebar out and back when the topbar asks', async () => {
        const wrapper = await mountApp();

        wrapper.findComponent(AppTopbar).vm.$emit('menu-toggle');
        await wrapper.vm.$nextTick();
        expect(wrapper.get('[data-testid="sidebar"]').classes()).toContain('-translate-x-full');

        wrapper.findComponent(AppTopbar).vm.$emit('menu-toggle');
        await wrapper.vm.$nextTick();
        expect(wrapper.get('[data-testid="sidebar"]').classes()).toContain('translate-x-0');
    });

    it('gives the main region the sidebar margin only while the sidebar is open', async () => {
        const wrapper = await mountApp();
        expect(wrapper.get('main').classes()).toContain('lg:ml-sidebar');

        wrapper.findComponent(AppTopbar).vm.$emit('menu-toggle');
        await wrapper.vm.$nextTick();
        expect(wrapper.get('main').classes()).not.toContain('lg:ml-sidebar');
    });

    it('mounts exactly one toast host', async () => {
        const wrapper = await mountApp();
        expect(wrapper.findAll('[data-testid="toaster"]').length).toBe(1);
    });
});
```

Note the first case depends on jsdom's default window width of 1024, which is *not* greater than 1024 — so implement the desktop check as `window.innerWidth >= 1024` and say so in the code, or the test fails for a reason that has nothing to do with the shell.

- [ ] **Step 2: Run them and watch them fail**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/AppMenu.spec.ts src/AppTopbar.spec.ts src/App.spec.ts'
```

Expected: all three suites fail — `AppMenu` takes a `model` prop and renders `AppSubmenu`, `AppTopbar` has no `sidebarOpen` prop and no `data-testid`, `App.vue` has no `[data-testid="sidebar"]`.

- [ ] **Step 3: Replace the menu model**

Replace the whole of `app/src/models/dashboard.ts` with:

```ts
import type {Component} from 'vue';

/**
 * One sidebar entry.
 *
 * The type this replaces carried twelve optional fields inherited from the
 * admin template it came from (`items`, `command`, `url`, `badge`,
 * `separator`, `target`, `class`, `style`, `disabled`, …). No menu entry has
 * ever set any of them, and the recursive submenu component that read them is
 * deleted in the same commit as this change.
 */
export type MenuEntry = {
    label: string;
    /** A lucide-vue-next icon component, rendered with <component :is>. */
    icon: Component;
    to: string;
};
```

- [ ] **Step 4: Rewrite `AppMenu.vue` and delete `AppSubmenu.vue`**

Replace the whole of `app/src/AppMenu.vue` with:

```vue
<script setup lang="ts">
import type {MenuEntry} from '@/models/dashboard';

defineProps<{items: MenuEntry[]}>();

const emit = defineEmits<{navigate: []}>();
</script>

<template>
  <nav class="pb-32" aria-label="Main">
    <ul class="m-0 list-none p-0">
      <li v-for="item in items" :key="item.to">
        <router-link
          :to="item.to"
          class="flex items-center gap-2 border-t border-sidebar-border p-4 text-sidebar-ink transition-colors hover:text-brand-light [&.router-link-active]:bg-sidebar-active [&.router-link-active]:text-sidebar-active-ink"
          @click="emit('navigate')">
          <component :is="item.icon" class="size-4 shrink-0" aria-hidden="true"/>
          <span>{{ item.label }}</span>
        </router-link>
      </li>
    </ul>
  </nav>
</template>
```

```bash
git rm app/src/AppSubmenu.vue
```

- [ ] **Step 5: Rewrite `AppTopbar.vue`**

Replace the whole of `app/src/AppTopbar.vue` with:

```vue
<script setup lang="ts">
import {computed} from 'vue';
import {Menu} from 'lucide-vue-next';
import {useAppStore} from '@/store/appStore';
import ProcessControl from '@/components/ProcessControl.vue';

defineProps<{sidebarOpen: boolean}>();

const emit = defineEmits<{'menu-toggle': []}>();

const appStore = useAppStore();
const settings = computed(() => appStore.getSettings);
</script>

<template>
  <header
    class="fixed inset-x-0 top-0 z-30 flex h-topbar items-center gap-4 bg-linear-to-r from-brand to-brand-light px-8 text-white transition-[left] duration-200"
    :class="sidebarOpen ? 'lg:left-sidebar' : ''">
    <button
      type="button"
      class="cursor-pointer text-white transition-colors hover:text-focus"
      aria-label="Toggle menu"
      data-testid="menu-toggle"
      @click="emit('menu-toggle')">
      <Menu class="size-6"/>
    </button>

    <div v-if="settings" class="ml-auto flex items-center gap-3">
      <span class="hidden sm:inline">Factorio with <strong>{{ settings.factorio.client_count }} Clients</strong></span>
      <ProcessControl/>
    </div>
  </header>
</template>
```

- [ ] **Step 6: Rewrite `AppFooter.vue`**

Replace the whole of `app/src/AppFooter.vue` with:

```vue
<template>
  <footer class="flex items-center gap-2 bg-card px-8 py-4 text-sm text-ink-muted transition-[margin] duration-200">
    <a class="text-link" target="_blank" rel="noopener noreferrer" href="https://github.com/arturh85/factorio-bot/">Factorio Bot</a>
    <img src="./assets/logo.png" alt="factorio-bot" width="20" height="20"/>
    <span>
      Built with
      <a class="text-link" target="_blank" rel="noopener noreferrer" href="https://vuejs.org/">Vue</a>,
      <a class="text-link" target="_blank" rel="noopener noreferrer" href="https://reka-ui.com/">Reka UI</a>
      and
      <a class="text-link" target="_blank" rel="noopener noreferrer" href="https://tailwindcss.com/">Tailwind CSS</a>
    </span>
  </footer>
</template>
```

The old credits named Tauri, PrimeVue and the Sigma Vue template. None of the three is in the app any more.

- [ ] **Step 7: Rewrite `App.vue`**

Replace the whole of `app/src/App.vue` with the following. Keep the `onMounted` body and the two-second poll from plan 5's Task 11 exactly as they are — this is a layout change, not a behaviour change.

```vue
<script setup lang="ts">
import {computed, onMounted, onUnmounted, ref, watch} from 'vue';
import {Cog, Home, Network, Terminal} from 'lucide-vue-next';
import AppTopbar from './AppTopbar.vue';
import AppMenu from './AppMenu.vue';
import AppFooter from './AppFooter.vue';
import Toaster from '@/components/ui/Toaster.vue';
import {useToast} from '@/composables/useToast';
import {useAppStore} from '@/store/appStore';
import {useInstanceStore} from '@/store/instanceStore';
import type {MenuEntry} from '@/models/dashboard';

const menu: MenuEntry[] = [
  {label: 'Dashboard', icon: Home, to: '/'},
  {label: 'Settings', icon: Cog, to: '/settings'},
  {label: 'RCON', icon: Terminal, to: '/rcon'},
  {label: 'LUA Script', icon: Terminal, to: '/script'},
  {label: 'Tasks', icon: Network, to: '/tasks'}
]

// jsdom reports exactly 1024, and so does a real 1024px viewport, which is the
// width Tailwind's `lg:` breakpoint starts at. Use the same comparison the
// breakpoint uses so the class toggles and the media query agree.
const isDesktop = () => window.innerWidth >= 1024

const sidebarOpen = ref(isDesktop())

function toggleSidebar() {
  sidebarOpen.value = !sidebarOpen.value
}

function onNavigate() {
  if (!isDesktop()) {
    sidebarOpen.value = false
  }
}

// An open overlay sidebar on a phone must not scroll the page behind it.
watch(sidebarOpen, open => {
  document.body.classList.toggle('overflow-hidden', open && !isDesktop())
})

const INSTANCE_POLL_MS = 2000
let instancePollTimer: number | null = null

const appStore = useAppStore()
const instanceStore = useInstanceStore()

onMounted(async () => {
  const toast = useToast()
  try {
    await appStore.loadSettings()
    await instanceStore.checkInstanceState()
  } catch (err) {
    toast.add({
      severity: 'error',
      summary: 'Cannot reach the factorio-bot server',
      detail: err instanceof Error ? err.message : String(err),
      life: 10000
    })
  }
  instancePollTimer = window.setInterval(() => {
    instanceStore.checkInstanceState().catch(() => {
      // A transient poll failure is not worth a toast every two seconds; the
      // next successful poll refreshes the state.
    })
  }, INSTANCE_POLL_MS)
})

onUnmounted(() => {
  if (instancePollTimer !== null) {
    window.clearInterval(instancePollTimer)
    instancePollTimer = null
  }
  document.body.classList.remove('overflow-hidden')
})

const sidebarClasses = computed(() => sidebarOpen.value ? 'translate-x-0' : '-translate-x-full')
const shiftedClasses = computed(() => sidebarOpen.value ? 'lg:ml-sidebar' : '')
</script>

<template>
  <div class="flex min-h-screen flex-col bg-surface text-ink">
    <AppTopbar :sidebar-open="sidebarOpen" @menu-toggle="toggleSidebar"/>

    <aside
      class="fixed inset-y-0 left-0 z-40 w-sidebar overflow-y-auto bg-sidebar shadow-[0_0_6px_0_rgba(0,0,0,0.16)] transition-transform duration-200"
      :class="sidebarClasses"
      data-testid="sidebar">
      <div class="mt-6 text-center">
        <router-link to="/">
          <img alt="Logo" src="./assets/logo.png" width="250"/>
        </router-link>
      </div>
      <AppMenu :items="menu" @navigate="onNavigate"/>
    </aside>

    <div
      v-if="sidebarOpen"
      class="fixed inset-0 top-topbar z-30 bg-black/70 lg:hidden"
      data-testid="sidebar-mask"
      @click="sidebarOpen = false"></div>

    <main class="flex-1 px-8 pb-8 pt-[70px] transition-[margin] duration-200" :class="shiftedClasses">
      <router-view/>
    </main>

    <AppFooter :class="shiftedClasses"/>
    <Toaster/>
  </div>
</template>
```

- [ ] **Step 8: Delete `AppConfig.vue` and the SCSS shell**

```bash
git rm app/src/AppConfig.vue
git rm -r app/src/assets/layout
```

In `app/src/main.ts`, delete the line:

```ts
import './assets/layout/layout.scss';
```

- [ ] **Step 9: Turn Preflight on and add the base layer**

In `app/src/assets/tailwind.css`, replace the leading comment and the two `@import` lines with:

```css
/* Tailwind v4 with Preflight.
 *
 * Preflight is Tailwind's global reset. It was held back while the SCSS layout
 * shell in assets/layout/ styled bare elements directly; that shell is gone as
 * of this commit, every page renders from utilities, and the reset is now what
 * keeps browser defaults out of the design. */
@layer theme, base, components, utilities;

@import "tailwindcss/theme.css" layer(theme);
@import "tailwindcss/preflight.css" layer(base);
@import "tailwindcss/utilities.css" layer(utilities);
```

and append, after the `@theme` block:

```css
/* The three element defaults the app genuinely relies on. Everything else is a
 * utility on the element that needs it -- headings included, which is why
 * Preflight zeroing heading sizes is not a problem any more. */
@layer base {
    html {
        font-size: 14px;
    }

    body {
        background-color: var(--color-surface);
        color: var(--color-ink);
        font-family: var(--font-sans);
        -webkit-font-smoothing: antialiased;
        -moz-osx-font-smoothing: grayscale;
    }
}
```

Preflight's effects to expect, all intended: bare `<h1>`-`<h6>` lose their sizes (every heading in the app now carries `text-xl` or similar from `Card.vue`); `<p>` loses its bottom margin (spacing is on the utilities); bare `<button>` loses its native chrome (all four button-like elements — the topbar hamburger, the Dashboard count, `Button.vue`, `Toggle.vue`, `TreeView.vue`'s rows — set their own); `<img>` becomes `display: block` (the footer logo is inside a flex row, so it is unaffected). Monaco injects its own stylesheet and is not touched by Preflight.

- [ ] **Step 10: Run the shell tests and watch them pass**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm vitest run src/AppMenu.spec.ts src/AppTopbar.spec.ts src/App.spec.ts'
```

Expected: 12 passed.

- [ ] **Step 11: Verify nothing still references the deleted shell**

```bash
grep -rn "AppSubmenu\|AppConfig\|assets/layout\|DashboardMenu\|layout-wrapper\|layout-topbar\|layout-sidebar\|layout-menu\|layout-main\|layout-footer\|class=\"card\"" app/src
```

Expected: no output.

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run lint && pnpm run test:coverage && pnpm run build:web'
```

Expected: all clean. The build is the real Preflight check — if the CSS fails to compile, the `@import` order is wrong.

- [ ] **Step 12: Look at it**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run serve'
```

Open `http://localhost:8080/#/`, click through all five routes, and check by eye: topbar gradient and hamburger; sidebar slides out and back and the active route is highlighted; Settings fields are aligned and full width (this page was the worst-broken one before); Script page splitter drags and the sizes survive a reload; a toast appears bottom-right when the Dashboard count is clicked and disappears after three seconds. Then narrow the window below 1024px and confirm the sidebar overlays with a mask instead of pushing the content.

- [ ] **Step 13: Commit**

```bash
git commit --only app/src/App.vue app/src/App.spec.ts app/src/AppTopbar.vue app/src/AppTopbar.spec.ts app/src/AppMenu.vue app/src/AppMenu.spec.ts app/src/AppFooter.vue app/src/AppSubmenu.vue app/src/AppConfig.vue app/src/models/dashboard.ts app/src/assets/tailwind.css app/src/assets/layout app/src/main.ts -m "feat(app): rebuild the layout shell in tailwind and enable preflight"
```

---

## Task 12: Remove PrimeVue, PrimeIcons and Sass

**Files:**
- Modify: `app/src/main.ts`, `app/package.json`
- Delete: `app/src/assets/logo_transparent.png` (unreferenced — Step 3 verifies before deleting)

**Interfaces:**
- Consumes: everything above.
- Produces: a `package.json` with no `primevue`, `@primeuix/themes`, `primeicons` or `sass` entry, and a `main.ts` that installs only Pinia and the router.

- [ ] **Step 1: Prove nothing imports PrimeVue any more**

```bash
grep -rn "primevue\|primeicons\|@primeuix\|\bpi-\|v-ripple\|v-tooltip" app/src
```

Expected: no output. Any hit is a component this plan did not migrate — stop and migrate it rather than deleting the dependency out from under it.

- [ ] **Step 2: Strip `main.ts`**

Replace the whole of `app/src/main.ts` with:

```ts
import {createApp} from 'vue';
import {createPinia} from 'pinia';
import router from './router';
import {useToast} from './composables/useToast';

import './assets/tailwind.css';

import App from './App.vue';

// Vue Router 5 deprecates the next() callback; returning undefined continues
// the navigation, and next() is removed entirely in Router 6.
router.beforeEach(() => {
    // Messages are about the page the user is leaving.
    useToast().clear();
    window.scrollTo(0, 0);
});

const app = createApp(App);

app.use(createPinia());
app.use(router);

app.mount('#app');
```

- [ ] **Step 3: Delete the unreferenced logo variant**

```bash
grep -rn "logo_transparent" app/src app/index.html
```

Expected: no output. Then:

```bash
git rm app/src/assets/logo_transparent.png
```

If the grep *does* find a reference, skip this step and leave the file.

- [ ] **Step 4: Remove the packages**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm remove primevue @primeuix/themes primeicons sass'
```

`sass` goes with them: after Task 11 there is no `.scss` file and no `<style lang="scss">` block left in `app/src`. Confirm before removing:

```bash
grep -rn "lang=\"scss\"" app/src; find app/src -name '*.scss'
```

Expected: no output from either.

- [ ] **Step 5: Full verification**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm install && pnpm run lint && pnpm run test:coverage && pnpm run build:web'
```

Expected: lint clean, all suites pass, coverage thresholds met, build succeeds.

Then check the bundle actually shrank — `dist/stats.html` is written by the existing `rollup-plugin-visualizer`:

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && ls -la dist/assets/*.css dist/assets/*.js | head'
```

Expected: no `primeicons` font files (`.woff2`, `.ttf`, `.eot`) in `dist/assets/` at all:

```bash
find app/dist -name '*primeicons*' -o -name '*.woff2' | head
```

Expected: no output.

- [ ] **Step 6: Run the app once more end to end**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cd app && pnpm run serve'
```

With `factorio-bot serve` running behind the Vite proxy, click through all five routes once more. This is the last chance to catch a style that was silently coming from PrimeVue's injected CSS rather than from a utility class.

- [ ] **Step 7: Commit**

```bash
git commit --only app/src/main.ts app/package.json app/pnpm-lock.yaml app/src/assets/logo_transparent.png -m "chore(app): remove primevue, primeicons and sass"
```

---

## Self-review

**Coverage against the audit — every PrimeVue component in use:**

| Audit item | Task | Note |
|---|---|---|
| `Button` (9 tags, 4 files) | 4 | `label` prop becomes slot content. |
| `Toast` + `useToast` (5 files) | 3 | Hand-written queue; `removeAllGroups()` → `clear()`, moved into `router.beforeEach`. |
| `InputText` (5) | 6 | |
| `Splitter` / `SplitterPanel` (2 / 4) | 8 | reka-ui, `auto-save-id` replaces `stateKey`/`stateStorage`. |
| `Checkbox` (3 → 2 after plan 5) | 6 | reka-ui. |
| `ToggleButton` (1) | 5 | Hand-written `aria-pressed` button. |
| `Textarea` (1) | 7 | |
| `Slider` (1) | 6 | reka-ui, with a `number` ↔ `number[]` adapter. |
| `Tree` (1) | 9 | Full rebuild: `TreeView.vue` + pure `mergeNodes()`. |
| `Ripple` directive | 11 | Its only two usages were in `AppSubmenu.vue`, deleted. |
| `Tooltip` directive | 12 | Registered globally, zero usages; dropped with the plugin. |
| `RadioButton` (6) | **none — deliberate** | All six were in `AppConfig.vue`. See below. |
| `ToggleSwitch` (1) | **none — deliberate** | Same. |

**Every dead-PrimeFlex file:** `EmptyPage.vue` and `GameInstances.vue` deleted (Task 2); `SettingsPage.vue` (Task 6); `RconPage.vue` (Task 7); `ScriptPage.vue` (Task 8); `Dashboard.vue` and `TasksPage.vue` (Task 10); `AppConfig.vue` deleted (Task 11). That is all nine, and Task 10 Step 6 greps for the whole class list to prove it. The SCSS-side dead selectors (`_config.scss:123`, `_dashboard.scss:88,103,109,161`) go with the directory in Task 11.

**Layout shell:** Task 11 rebuilds all fourteen partials' live rules as utilities — `_main`, `_topbar`, `_sidebar`, `_menu`, `_content`, `_footer`, `_responsive`, `_utils` (`.card` → `Card.vue`), `_typography` (per-element utilities plus the Preflight reset), `_dashboard` (Task 10) — and drops the four that styled nothing: `_splash.scss` (no splash element exists), `_profile.scss` (no profile widget was ever ported), `_config.scss` (drawer deleted) and `_mixins.scss` (its consumers are gone).

**Gaps left deliberately, and why:**

1. **`RadioButton` and `ToggleSwitch` get no replacement.** Their only consumer, `AppConfig.vue`, is deleted in Task 11: two of its four controls have empty handlers and do nothing today, and the other two are admin-template theming preferences that this redesign settles once (pinned-on-desktop sidebar, one sidebar palette). If a radio group or a switch is wanted later, `reka-ui`'s `RadioGroupRoot`/`RadioGroupItem` and `SwitchRoot`/`SwitchThumb` are already installed.
2. **`GanttChart.vue` stays a `<div>TODO</div>` stub.** Filling it in is planner work with no design input available; Task 10 restyles its container only.
3. **`Editor.vue` (Monaco) is not touched.** It is not a PrimeVue component and its worker wiring is orthogonal to this migration. Tasks 8 and 12 both re-run `build:web`, which is what would catch a Vite-side breakage.
4. **No tests are added for `Editor.vue`, `GanttChart.vue`, `ProcessControl.vue`, `AppFooter.vue` or `main.ts`.** This plan is not a testing initiative; the coverage gate covers what it creates (`src/lib`, `src/composables`, `src/components/ui`), and the page-level specs it adds are extra rather than gated.
5. **No dark mode.** The audit records `darkModeSelector: false` — the shell was a hard-coded light theme and stays one. The tokens are CSS custom properties, so a dark palette is a later `@media (prefers-color-scheme: dark)` block over the same names.
6. **Accessibility is improved but not audited.** Every interactive element this plan writes carries a role, a name and a focus ring, but no keyboard-navigation pass over the tree (arrow keys move focus in PrimeVue's `Tree`; `TreeView.vue` gives each row a native button and relies on Tab). Worth a follow-up, not a blocker.
