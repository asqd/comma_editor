APP := build/DerivedData/Build/Products/Debug/CommaEditor.app

.PHONY: test core xcodeproj app run clean

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

clean:
	cargo clean
	rm -rf build apps/macos/CommaCore apps/macos/CommaEditor.xcodeproj
