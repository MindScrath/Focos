.PHONY: all build fmt lint test smoke check clean

TARGET := riscv64gc-unknown-none-elf
HOST_TARGET := $(shell rustc -vV | sed -n 's/^host: //p')

all: build

build:
	cargo build --locked --release --target $(TARGET)

fmt:
	cargo fmt --all -- --check

lint:
	cargo clippy --locked --release --target $(TARGET) -- -D warnings
	cargo clippy --locked -p focos-kernel --lib --target $(HOST_TARGET) -- -D warnings

test:
	cargo test --locked -p focos-kernel --lib --target $(HOST_TARGET)

smoke: build
	python3 scripts/smoke.py

check: fmt lint test smoke

clean:
	cargo clean
