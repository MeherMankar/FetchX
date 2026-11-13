# FetchX Android Browser UI/UX Specification

Status: research and design specification only  
Scope: Compose/browser presentation and interaction design  
Out of scope: browser/session architecture, Rust, bridge, download engine behavior, and
implementation code

## 1. Executive Summary

FetchX should present a calm, download-first browser built around:

> Browse -> Download -> Manage

The address/search field remains at the top at all times. Bottom controls are reserved
for navigation and high-frequency actions. The design should reduce the current menu
density, make downloads visible without becoming intrusive, and preserve the existing
`BrowserScreen -> BrowserViewModel -> TabManager -> GeckoView/GeckoSession` boundary.

The first implementation phase should be visual restructuring only. Existing callbacks,
state flows, persistence, Gecko delegates, and bridge contracts remain the behavior
boundary.

## 2. Current FetchX Baseline

### Observed

- `BrowserScreen` is a Compose screen containing the top toolbar, GeckoView, overlays,
  dialogs, tab overview, and bottom browser bar.
- `BrowserViewModel` exposes `BrowserUiState` and coordinates browser presentation actions.
- `TabManager` owns Gecko sessions and emits immutable `TabState` snapshots.
- `BrowserScreen` uses `AndroidView` to attach a `GeckoView`.
- Tabs, bookmarks, history, privacy settings, and selected browser presentation settings
  are persisted through the existing ViewModel/store paths.
- `BridgeDownloadRepository` is the Android boundary to the UniFFI `DownloadService`.
- Browser external responses are confirmed in a FetchX download dialog before being sent
  to the repository.

Evidence:

- [BrowserScreen.kt](../apps/android/browser/src/main/kotlin/dev/fetchx/browser/BrowserScreen.kt)
- [BrowserViewModel.kt](../apps/android/browser/src/main/kotlin/dev/fetchx/browser/BrowserViewModel.kt)
- [TabManager.kt](../apps/android/browser/src/main/kotlin/dev/fetchx/browser/TabManager.kt)
- [TabState.kt](../apps/android/browser/src/main/kotlin/dev/fetchx/browser/TabState.kt)
- [AppNavigation.kt](../apps/android/app/src/main/kotlin/dev/fetchx/app/navigation/AppNavigation.kt)
- [BridgeDownloadRepository.kt](../apps/android/bridge/src/main/kotlin/dev/fetchx/bridge/BridgeDownloadRepository.kt)

### Known behavior gaps

The existing audit found no proven implementation for share, find in page, desktop site,
real Gecko tab thumbnails, or a dedicated private-tab model. The loading action calls
GeckoView's `GeckoSession.stop()` while a page is loading, while reload remains the idle
action. Security presentation is limited to URL-scheme and internal-page indicators;
certificate inspection is not part of this UI scope.

## 3. Research References

The following references were studied at the repository/product-documentation level.
Where a visual detail was not directly verifiable from an official source, it is marked
`NOT VERIFIED` rather than inferred.

### WebLibre

**Observed:** WebLibre describes a privacy-focused Android browser with browsing
separation and deep customization. Its project material emphasizes configurable toolbar
actions, containers, tab organization, and adaptive tab presentation.

**Useful idea:** Make tab context and privacy context visible without requiring users to
open a large undifferentiated menu.

**FetchX decision:** ADAPT. Use FetchX's existing Spaces/profile identity and tab overview,
but keep the address bar at the top and avoid adding a new tab/session model.

**Architecture fit:** Restyle `BrowserTabStrip`, `TabGridSheet`, and the existing profile
dialogs. Do not change `TabManager` or `TabStore`.

Reference: [WebLibre repository](https://github.com/FaFre/WebLibre)

### FOSS Browser

**Observed:** FOSS Browser emphasizes Material You styling, one-handed use, bottom
controls, gesture customization, profiles, and per-site privacy controls.

**Useful idea:** Reachable controls, predictable touch targets, and a compact action
surface.

**FetchX decision:** ADAPT selectively. Use the spacing, gesture, and reachability
principles, but REJECT moving the address bar to the bottom because FetchX explicitly
requires top address/search.

**Architecture fit:** Apply to `BrowserBottomBar`, sheets, and tab gestures. Gestures
must remain UI event routing to existing callbacks.

Reference: [FOSS Browser on F-Droid](https://f-droid.org/packages/de.baumann.browser/)

### IronFox

**Observed:** IronFox is a hardened Firefox-derived browser whose product identity is
privacy and security. Its documented features include strict tracking protection,
HTTPS-only mode, uBlock Origin, permission restrictions, and clearing controls.

**Useful idea:** Privacy should be understandable through strong defaults and a concise
security surface, not through exposing every internal setting in the first view.

**FetchX decision:** ADAPT. Keep a compact security summary in the address bar and put
advanced privacy controls behind a secondary surface. Do not copy Firefox branding or
adopt undocumented behavior.

**Architecture fit:** Restyle the existing security indicator, tracker details, and
privacy dialogs. Preserve `PrivacyController` and Gecko runtime configuration.

Reference: [IronFox repository](https://github.com/ironfox-oss/IronFox)

### Lightning Browser

**Observed:** Lightning presents itself around speed, simplicity, and security. Its
documented features include bookmarks, history, multiple search engines, incognito mode,
search suggestions, and a navigation drawer or bottom drawer for tabs.

**Useful idea:** Keep the primary browsing surface small and make secondary collections
easy to reach.

**FetchX decision:** ADAPT. Use a compact tab/download action model and clear empty
states. Do not infer the current visual layout beyond the documented feature set.

**Architecture fit:** Apply to the browser menu, essentials page, and empty states.

Reference: [Lightning Browser repository](https://github.com/anthonycr/Lightning-Browser)

### Dejavu

**Observed:** Dejavu presents workspaces, containers, split view, and tab organization as
first-class concepts. Its project description emphasizes keeping tabs in their context.

**Useful idea:** A workspace/profile indicator can reduce tab confusion without requiring
new tab grouping.

**FetchX decision:** ADAPT the visible identity only. Existing FetchX Spaces/profiles can
show color and name in the tab overview and menu. REJECT introducing containers, split
view, folders, or tab groups in this UI-only phase.

**Architecture fit:** Use existing `spaceId`, profile state, and callbacks.

Reference: [Dejavu repository](https://github.com/by-architect/Dejavu)

## 4. Design Principles

1. **Top address/search:** The address bar never moves into bottom navigation.
2. **Browse -> Download -> Manage:** Downloads are visible and reachable, but never
   obscure page content without a user action.
3. **One clear primary action:** Each surface should have one obvious next step.
4. **Progressive disclosure:** Common actions are visible; advanced actions are grouped.
5. **Privacy at a glance:** Show secure/insecure and tracker status simply, with detail on
   demand.
6. **GeckoView-friendly:** Compose owns chrome; GeckoView owns page content.
7. **State-driven presentation:** Existing `BrowserUiState` remains the source for browser
   chrome.
8. **No silent behavior changes:** A new visual affordance must map to an existing
   callback or be labeled future work.
9. **Comfortable for long sessions:** Avoid excessive motion, dense labels, and repeated
   modal interruption.

## 5. FetchX Visual Identity

FetchX should be recognizable as a utility browser rather than a clone of Chrome,
Firefox, Samsung Internet, or Brave:

- Deep neutral surfaces with one configurable accent color.
- A restrained download accent, used for active/download affordances only.
- Security shown as a compact shield/lock state rather than a large security panel.
- Rounded Material 3 surfaces, but not every control rendered as a floating pill.
- Strong text hierarchy and generous vertical spacing.
- Download status treated as a first-class utility signal.

The existing profile accent and theme seed should drive these colors instead of adding
component-specific hard-coded palettes.

## 6. Main Browser Screen

### Recommended layout

```text
┌────────────────────────────────────┐
│ Optional tab context / active tab  │
├────────────────────────────────────┤
│ [security]  example.com     [menu] │
│             address/search         │
├────────────────────────────────────┤
│ loading progress                   │
├────────────────────────────────────┤
│                                    │
│              GeckoView             │
│                                    │
│  error/security/download overlays  │
│                                    │
├────────────────────────────────────┤
│ Back  Forward  Home  Tabs  Download│
└────────────────────────────────────┘
```

### Recommendations

- Hide the horizontal tab strip when there is one tab.
- Show a compact tab-context row when there are multiple tabs or when the active
  workspace/profile changes.
- Keep the address row approximately 52-60 dp before insets.
- Keep all interactive controls at least 48 dp touch targets.
- Use a thin progress indicator directly below the address row.
- Keep GeckoView as the largest region.
- Use a transient download indicator above the bottom bar only when a download event is
  pending or active.
- Preserve status-bar padding above the address row and safe-drawing padding below the
  bottom bar.

### Bottom bar recommendation

Permanent actions:

1. Back
2. Home
3. Tabs
4. Downloads
5. Menu

Forward is a secondary action in the menu or an adaptive slot beside Back when forward
history exists. This reduces permanent visual density while preserving access.

The existing bottom bar should not contain the address field.

## 7. Address Bar Specification

### Collapsed state

```text
┌────────────────────────────────────┐
│ [shield]  github.com       [reload]│
└────────────────────────────────────┘
```

- Show the security icon at the leading edge.
- Display the host prominently; retain enough path context to disambiguate pages when
  useful.
- Truncate long values in the middle, not only at the end.
- Keep reload/stop and bookmark as trailing actions only when space permits.
- Keep overflow/menu outside the editable field.
- Use the profile accent for the focused border or subtle highlight.
- Do not show a false “secure” state for non-HTTPS URLs.

### Editing state

```text
┌────────────────────────────────────┐
│ [back] https://github.com/... [×]  │
├────────────────────────────────────┤
│ GitHub                             │
│ github.com                         │
│ Search the web for "..."           │
└────────────────────────────────────┘
```

- Select all existing text when editing begins, matching current FetchX behavior.
- Keep the search-engine selector at the leading edge of the field.
- Keep clear text at the trailing edge.
- Submit from IME Go, the explicit navigation button, or a suggestion.
- Reuse `BrowserViewModel.submitSearch()` and the existing classifier.
- Do not introduce a second URL parser or search-routing implementation.
- Debounce suggestions and show a small progress indicator only while loading.
- Limit the visible suggestion list to a short, scannable set.
- Clearly distinguish local bookmark/history/tab results from web search.
- Close editing with the system Back action without navigating.

### Security interaction

Tapping the security icon opens a compact sheet:

```text
Security
──────────────
✓ Secure connection
Trackers blocked: 12

Privacy settings
Site details
```

The first version should use the existing tracker and privacy callbacks. Certificate
inspection and site-permission management are `FUTURE IDEA` unless existing APIs are
confirmed.

## 8. Bottom Navigation

### Primary actions

- Back: enabled only when `canGoBack`.
- Home: navigates to the existing `about:blank` essentials page.
- Tabs: opens the existing tab overview.
- Downloads: navigates to the existing Downloads destination.
- Menu: opens the browser action sheet.

### Secondary actions

- Forward.
- Reload.
- Bookmark.
- Reader mode.
- Share.
- Find in page.
- Desktop site.

Share, find in page, and desktop site are `FUTURE IDEA` until their GeckoView
implementations are defined. Do not render them as working actions prematurely.

### State behavior

- Disabled navigation uses reduced contrast, not removal, so layout does not jump.
- Loading shows progress under the address bar.
- The reload affordance must not be labeled “Stop” unless a real stop callback exists.
- Active downloads show a small count/progress badge on Downloads.
- Tab count is shown on the Tabs button and remains legible at 1-99; larger counts use a
  bounded representation.

## 9. Tab System

### Tab button

- Show count and current workspace accent.
- Use a 48 dp minimum touch target.
- Long press may be reserved for future tab actions; do not add behavior without a
  defined callback.

### Tab overview

Recommended default: grid on phones, adaptive two-column or wider grid on large screens.
Offer a list presentation only if it improves accessibility or search scanning.

Each card contains:

- Page title or host.
- Compact URL.
- Workspace/profile accent.
- Loading/error indicator.
- Close button.
- Active-state border.

Real thumbnails are `FUTURE IDEA`. Current `TabState` contains no screenshot, and adding
thumbnail capture would cross the current UI-only boundary.

### Existing architecture fit

- Create/select/close through existing ViewModel methods.
- Keep `TabManager` as GeckoSession owner.
- Keep `TabStore` as the tab snapshot store.
- Use existing `spaceId` for visual filtering.
- Do not add tab groups, containers, split view, or reopen-closed-tab persistence in this
  phase.

### Private tabs

`NOT VERIFIED` in the current FetchX implementation. The design may reserve a future
visual slot for private mode, but must not show a private-tab control until a distinct
session/state contract exists.

## 10. New-Tab / Essentials Page

### Start-page hierarchy

```text
┌────────────────────────────────────┐
│ FetchX                 [profile]   │
│      Search or type URL            │
│   Downloads · live queue count     │
│ Top sites                          │
│ (site)  (site)  (site)  (site)     │
│ Bookmarks                 History  │
└────────────────────────────────────┘
```

### Rules

- Visual reference: [Brave Browser UI kit](https://www.figma.com/community/file/1032081846581442397/brave-browser-ui-kit) and
  [Brave's Android New Tab Page](https://support.brave.app/hc/en-us/articles/36070347466509-How-do-I-customize-the-New-Tab-Page-on-Android).
- Use a recognizable browser start-page rhythm: brand, search, one compact live
  status card, then a horizontal top-sites strip.
- The central search affordance opens the existing address editor; it does not create
  a separate search flow or duplicate URL state.
- Keep the download card factual and driven by the current queue. Do not invent privacy
  counters or show zero-value metrics as decoration.
- Use the existing essentials/most-visited data.
- Keep the profile/space identity visible but compact.
- Show a clear empty state when there are no history/bookmark/essential entries.
- Keep Bookmarks and History as quiet secondary shortcuts; Tabs remain in the browser bar.
- Do not add news feeds, sponsored content, weather, or unrelated personalization.
- Personalization is a future option limited to ordering, accent, and shortcut visibility.

## 11. Browser Menu

Replace the current long undifferentiated sheet with grouped sections:

### Navigation

- Back
- Forward
- Reload

### Tabs

- New tab
- Tabs overview
- Current space/profile

### Page

- Bookmark/save
- Reader mode
- Download current page
- Future: Find in page
- Future: Share
- Future: Desktop site

### Privacy

- Tracking protection
- Privacy and security
- Site details, if supported by an existing API

### FetchX

- Downloads
- Extensions
- Storage
- Browser settings
- Appearance

Use a compact quick-action row only for the three most frequent actions. Avoid repeating
all bottom-bar controls in a large icon grid and a long list unless the action has a
meaningful secondary option.

## 12. Download UX

### Browser response download

**Observed:** GeckoView external responses are converted into a pending download, then
confirmed through a FetchX dialog before `BridgeDownloadRepository.addBrowserDownload()`.

**Recommended presentation:**

- Keep the confirmation dialog, but show filename, source host, and destination category.
- Use one primary action: “Download with FetchX”.
- Use a secondary “Cancel”.
- Explain that cancellation leaves the browser unable to continue the response only when
  that limitation is accurate.

### Media detection

**Observed:** A video detection banner opens a quality/source picker.

**Recommended presentation:**

- Keep the banner transient and dismissible.
- Show one active detection summary, with a count for multiple detections.
- Show source, quality, and estimated size when available.
- Use “Download” as the primary action and retain the existing media queue callback.

### Persistent download visibility

- Add a small Downloads badge for active/queued items.
- On completion or failure, use a concise snackbar or notification deep-link.
- Do not duplicate the entire download list inside the browser.
- Keep lifecycle, queue, retry, pause, cancel, and persistence in the existing download
  architecture.

## 13. Privacy and Security UX

Default address-bar presentation:

```text
[shield] example.com
```

Tap opens:

```text
Security
──────────────
✓ Secure connection
Trackers blocked: 12

Privacy settings
```

States:

- HTTPS: secure icon and “Secure connection”.
- HTTP or unknown: neutral/insecure icon and concise warning.
- Tracker protection active: shield plus blocked count when nonzero.
- Cloudflare bypass: transient status banner with a clear failure/dismiss action.

Do not expose raw Gecko categories, internal controller names, or implementation details
in the default surface.

## 14. Error and Empty States

Use a common pattern:

```text
Icon
Title
Short explanation
Primary action
Optional secondary action
```

| State | Title | Primary action | Secondary action |
|---|---|---|---|
| Offline/network error | “Couldn’t load this page” | Retry | Go home |
| Cloudflare/degraded page | “Security check in progress” | Wait/try again | Dismiss |
| Download failure | “Download failed” | Retry | Open Downloads |
| Bridge unavailable | “Downloads unavailable” | Retry initialization/restart | Continue browsing |
| Empty tab | “Start browsing” | Focus address bar | Open Downloads |
| No history | “No history yet” | Browse the web | Close |
| No bookmarks | “No bookmarks yet” | Bookmark current page | Close |
| No downloads | “No downloads yet” | Add a download | Close |

Use existing error state and callback paths. New retry actions require existing retry
methods or a separately approved logic change.

## 15. Animation Guidelines

| Trigger | Motion | Duration | Purpose | Reduced motion |
|---|---|---:|---|---|
| Address bar focus | Toolbar expands, suggestions fade/slide in | 150-200 ms | Establish editing mode | Crossfade |
| Tab opens | Card enters with short fade/scale | 150 ms | Confirm creation | Fade only |
| Tab closes | Card slides toward close direction | 150 ms | Preserve spatial context | Immediate removal |
| Menu opens | Bottom sheet rises | Material default | Maintain sheet affordance | Fade |
| Download detected | Banner rises from bottom | 200 ms | Make detection noticeable | Fade |
| Page loading | Thin progress grows under toolbar | Browser-driven | Show activity without blocking | Keep progress |
| Error appears | Banner slides down | 180 ms | Connect error to page | Fade |
| New tab | Essentials fades in after GeckoView is ready | 150 ms | Avoid flash | Immediate |

Animations must not delay navigation or download actions. Respect Android reduced-motion
settings where available. Do not add production sleeps or timing-dependent behavior.

## 16. Material 3 Design System

### Typography

- Display/headline: reserved for empty-state or full-screen titles.
- Title: sheet/dialog titles and section headings.
- Body: explanations and page metadata.
- Label: toolbar labels, badges, and compact controls.

Use Material 3 typography tokens rather than per-component arbitrary sizes.

### Shapes

- Address bar: 20-24 dp radius.
- Compact controls: 10-14 dp radius.
- Tab cards: 16 dp radius.
- Bottom sheets: 24 dp top corners.
- Dialogs: Material 3 default shape.

### Spacing

Use a 4 dp base grid:

- 4 dp: icon-to-label micro spacing.
- 8 dp: compact row spacing.
- 12 dp: control padding.
- 16 dp: standard surface padding.
- 24 dp: section separation.
- 32 dp: major empty-state separation.

### Colors

Use theme tokens and the existing profile seed:

- Primary: profile/theme accent.
- Surface: browser background.
- Surface container: toolbar, cards, and sheets.
- On-surface: primary text.
- On-surface-variant: secondary text.
- Error: load/download failure.
- Success: completed download and secure confirmation.
- Warning: insecure/attention state.
- Download: accent-derived but distinguishable from security.

Avoid adding isolated literal colors throughout `BrowserScreen`.

### Icons

Use Material icons with content descriptions. Icons must not be the only indication of
state; pair security, loading, download, and error icons with text or accessible labels.

## 17. Phone vs Tablet

### Phone

- Keep top address row and bottom action bar.
- Use a full-screen tab overview.
- Use one-column essentials and menu surfaces.
- Keep primary targets within comfortable thumb reach.

### Tablet / large screen

- Keep address/search at the top.
- Use a wider tab strip or two-column tab overview.
- Allow the menu to use a constrained side or centered sheet rather than full width.
- Use multi-column essentials shortcuts.
- Preserve the same callbacks, state, and screen architecture.

Use responsive Compose layout decisions, not separate browser implementations.

## 18. Accessibility

- Minimum 48 dp touch targets.
- Meaningful content descriptions for every icon-only control.
- Announce loading, download completion, and error states.
- Preserve focus when opening and closing the address editor.
- Support dynamic font scaling without clipping URLs or labels.
- Maintain contrast for secure, warning, error, and disabled states.
- Do not encode state by color alone.
- Make tab cards and close buttons separately focusable.
- Ensure keyboard/DPAD navigation can reach address bar, tabs, menu, and downloads.
- Respect reduced-motion settings.
- Provide text equivalents for tab counts, blocked counts, and download progress.

## 19. Existing Component Mapping

| Component | Classification | Direction |
|---|---|---|
| `BrowserScreen` | RESTRUCTURE UI | Keep orchestration and callbacks; simplify layout |
| `BrowserViewModel` | DO NOT TOUCH | Preserve state and action contracts |
| `BrowserBottomBar` | RESTYLE | Reduce density; keep address bar out |
| Address-bar block in `BrowserScreen` | RESTYLE / RESTRUCTURE UI | Preserve `submitSearch`, suggestions, focus, and URL sync |
| `BrowserMenuSheet` | RESTRUCTURE UI | Group actions and reduce duplication |
| `TabGridSheet` | RESTYLE | Improve cards, spacing, responsive layout |
| `EssentialsPage` | RESTYLE | Improve hierarchy and empty states |
| Error banners | RESTYLE | Adopt common icon/title/action pattern |
| Download confirmation UI | RESTYLE | Make source, filename, and primary action clearer |
| Video detection UI | RESTYLE | Preserve detection and queue callbacks |
| `TabManager` | DO NOT TOUCH | Session ownership and Gecko callbacks |
| `TabStore` / `BrowserDataCodec` | DO NOT TOUCH | Persistence and state representation |
| `GeckoRuntimeHolder` | DO NOT TOUCH | Runtime and extension ownership |
| `BridgeDownloadRepository` | DO NOT TOUCH | Rust/Android download boundary |
| Rust download system | DO NOT TOUCH | Queue, engine, persistence, and lifecycle |

## 20. Implementation Boundary

| Area | UI-only change? | Logic change? | Risk |
|---|---:|---:|---|
| Address-bar appearance | Yes | No | Medium |
| Address-bar focus animation | Yes | No | Low |
| Bottom bar layout | Yes | No | Low |
| Tab switcher presentation | Yes | No | Medium |
| Menu grouping | Yes | No | Low |
| Essentials layout | Yes | No | Low |
| Download prompt presentation | Yes | No | Medium |
| Video detection presentation | Yes | No | Medium |
| GeckoView | No | No | High |
| `TabManager` | No | No | High |
| `BrowserViewModel` | No | No | High |
| Tab persistence | No | No | High |
| Bridge repository | No | No | Critical |
| Rust queue/engines | No | No | Critical |

“UI-only” still requires preserving callback identity, state meaning, accessibility, and
existing navigation behavior.

## 21. Recommended Implementation Phases

### Phase 1: Chrome hierarchy

- Restyle top address row.
- Keep address/search at the top.
- Simplify bottom bar.
- Correct insets and touch targets.

### Phase 2: Menu and action grouping

- Reorganize `BrowserMenuSheet`.
- Remove duplicate primary actions where appropriate.
- Clearly mark unavailable/future actions rather than wiring placeholders.

### Phase 3: Tabs

- Restyle `TabGridSheet`.
- Improve active/workspace identity.
- Add responsive phone/tablet layout.
- Keep metadata cards; do not add screenshot capture yet.

### Phase 4: Essentials and states

- Restyle `EssentialsPage`.
- Standardize error, empty, loading, and Cloudflare presentation.

### Phase 5: Download-first polish

- Improve browser download confirmation.
- Add download count/progress affordance using existing state.
- Improve media detection banner and picker.

### Phase 6: Accessibility and validation

- Add Compose/UI tests for the browser shell.
- Validate dynamic font scale, TalkBack labels, keyboard navigation, and reduced motion.
- Validate with real GeckoView/device scenarios before claiming runtime behavior.

### Later, separately approved features

- Real tab thumbnails.
- Private tabs.
- Find in page.
- Share.
- Desktop site.
- Reopen closed tabs.
- Tab groups/containers.
- Split view.

## 22. Explicitly Do Not Change

- `BrowserViewModel.attachView()`.
- `TabManager.buildSession()` and Gecko delegates.
- `GeckoRuntimeHolder` ownership.
- `BridgeDownloadRepository` and UniFFI boundaries.
- Rust queue, downloader, torrent, and persistence architecture.
- Magnet interception behavior.
- External-response interception behavior.
- Privacy/tracker-blocking configuration.
- Tab persistence format and `BrowserDataCodec`.
- Download state mapping.
- Address/search classification logic.
- Existing download destination and publication behavior.

## 23. Research Confidence and Unknowns

- Official repository-level product descriptions verified the broad patterns for the
  reference browsers.
- Exact current pixel layouts, animations, and device-specific behavior for all references
  are `NOT VERIFIED` without installing and observing each release build.
- FetchX real-device GeckoView behavior is outside this document and must be validated in
  the implementation phase.
- No recommendation in this document is a claim that a missing FetchX feature is already
  implemented.
