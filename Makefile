APP := build/DerivedData/Build/Products/Debug/CommaEditor.app

.PHONY: test core xcodeproj app run icon release clean

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
		-configuration Debug -derivedDataPath build/DerivedData build

run: app
	open $(APP)

# Redraws the app icon from design/make-icon.swift.
icon:
	./scripts/make-icon.sh

# Universal release build and DMG; see docs/RELEASE.md for signing and notarization.
release:
	./scripts/release.sh

clean:
	cargo clean
	rm -rf build apps/macos/CommaCore apps/macos/CommaEditor.xcodeproj
