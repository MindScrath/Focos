.PHONY: all build fmt lint smoke check vf2 vf2-check clean

TARGET := riscv64gc-unknown-none-elf

all: build

build:
	cargo build --locked --release --target $(TARGET)

fmt:
	cargo fmt --all -- --check

lint:
	cargo clippy --locked --release --target $(TARGET) -- -D warnings

smoke: build
	python3 scripts/smoke.py

check: fmt lint smoke

vf2:
	cargo build --locked --release --target $(TARGET) --features vf2-bootelf --target-dir target/vf2
	cp target/vf2/$(TARGET)/release/focos-kernel target/vf2/focos-vf2.elf
	python3 scripts/smoke_bootelf.py --check-elf-only
	sha256sum target/vf2/focos-vf2.elf

vf2-check: fmt vf2
	cargo clippy --locked --release --target $(TARGET) --features vf2-bootelf --target-dir target/vf2 -- -D warnings
	python3 scripts/smoke_bootelf.py

clean:
	cargo clean
