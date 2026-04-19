# Apple app (iOS / iPadOS / macOS)

Xcode Multiplatform SwiftUI template, one target family, Universal Purchase. Substantial per-platform view divergence — not just `#if os` sprinkles. Will be added in Phase 3.

## Expected layout

```
apple/
├── ChessTutor.xcodeproj
├── Shared/            Shared SwiftUI view models, services, board rendering primitives
├── iOS/               Touch-first scene: board + slide-under analysis panel
├── macOS/             Desktop scene: sidebar move list, menus, keyboard shortcuts, multi-window
└── Frameworks/
    └── ChessTutorCore.xcframework     built by scripts/build-xcframework.sh
```

## Layout notes

- iOS layout: board is hero; analysis slides in from the bottom on demand; per-move feedback appears as a non-blocking chip under the board.
- macOS layout: sidebar move list on the left, board centre, analysis pane on the right, menu-bar actions (New Game, Import PGN, Analyse…), keyboard shortcuts for move navigation, multi-window support for "review game" alongside "active game".
- Shared: view models, services, board rendering primitives, colour/theme, drag-drop move handling.

## Bundle ID

`com.unbrokentechnology.chesstutor` — same ID for both iOS and macOS targets. That's what makes Universal Purchase work.

## App Store Connect

- Apple Developer Program: active (enrolled February 2026).
- Before the first TestFlight build: register the App ID in the Developer Portal and enable **Universal Purchase** in App Store Connect. Retrofitting later is painful.
- macOS is reached via SwiftUI platform conditionals, not Catalyst.
- "Designed for iPad" on Apple Silicon Macs gives free Mac smoke-testing during early iOS work.
