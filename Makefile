.PHONY: all build fmt lint test smoke mm-smoke irq-smoke vm-smoke timebase-smoke timer-smoke irq-timer-smoke trap-negative-smoke check clean

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
	python3 -B -m unittest discover -s scripts -p 'test_*.py'

smoke: build
	python3 scripts/smoke.py

mm-smoke: build
	python3 scripts/smoke.py --mm

irq-smoke: build
	python3 scripts/smoke.py --irq

vm-smoke: build
	python3 scripts/smoke.py --vm

timebase-smoke: build
	python3 scripts/smoke.py --timebase

timer-smoke: build
	python3 scripts/smoke.py --timer

irq-timer-smoke: build
	python3 scripts/smoke.py --irq-timer

trap-negative-smoke:
	python3 -B scripts/trap_negative.py

check: fmt lint test smoke

clean:
	cargo clean
