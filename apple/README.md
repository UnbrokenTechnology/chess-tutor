# Apple app (iOS / iPadOS / macOS)

SwiftUI, single target, Universal Purchase. Will be added in Phase 3.

## Expected layout

```
apple/
├── ChessTutor.xcodeproj
├── Shared/            SwiftUI views, view models, board rendering
├── iOS/               iOS-specific entry point + assets
├── macOS/             macOS-specific entry point + assets (no Catalyst)
└── Frameworks/
    └── ChessTutorCore.xcframework     built by scripts/build-xcframework.sh
```

## Notes

- **Universal Purchase must be enabled in App Store Connect before the first TestFlight build.** Retro-fitting it is painful.
- macOS is reached via SwiftUI platform conditionals, not Catalyst.
- "Designed for iPad" on Apple Silicon Macs gives free Mac smoke-testing during early iOS work.
