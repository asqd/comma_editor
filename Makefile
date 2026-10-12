APP := build/DerivedData/Build/Products/Debug/CommaEditor.app

.PHONY: test core xcodeproj app run icon release desktop desktop-build desktop-release version clean

# Rust tests: parsing, detection, byte-exact round-trips, editing, undo.
test:
	cargo test --workspace

# Static library + Swift bindings for the macOS app (apps/macos/CommaCore).
core:
	./scripts/build-core.sh

xcodeproj: core
	cd apps/macos && xcodegen generate

app: xcodeproj
	xcodebuild -project apps/macos/CommaEditor.xcodeproj -scheme CommaEditor \
		-configuration Debug -destination "platform=macOS,arch=$$(uname -m)" \
		-derivedDataPath build/DerivedData -quiet build

run: app
	open $(APP)

# Redraws the app icon from design/make-icon.swift.
icon:
	./scripts/make-icon.sh

# Universal release build and DMG; see docs/RELEASE.md for signing and notarization.
release:
	./scripts/release.sh

# The desktop app on Tauri for Windows, Linux and macOS (apps/desktop); needs Node 22.
desktop:
	cd apps/desktop && { [ -d node_modules ] || npm ci; } && npx tauri dev

# Release build of the desktop app, without installers:
# apps/desktop/src-tauri/target/release/commaeditor
desktop-build:
	cd apps/desktop && { [ -d node_modules ] || npm ci; } && npx tauri build --no-bundle

# Installers of the desktop app for this system, with their SHA-256, in build/desktop:
# a universal DMG on macOS, an NSIS installer on Windows, an AppImage and a .deb on Linux.
desktop-release:
	./scripts/desktop-release.sh

# The version of every build; `make version VERSION=1.1.0` sets it.
version:
	./scripts/version.sh $(VERSION)

clean:
	cargo clean
	rm -rf apps/desktop/src-tauri/target apps/desktop/dist
	rm -rf build apps/macos/CommaCore apps/macos/CommaEditor.xcodeproj
