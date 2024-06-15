.PHONY: clean outdated print test

default: test

clean:
	rm -rf Cargo.lock target/

outdated:
	cargo-outdated

print:
	git status --porcelain

test:
	cargo build --workspace

	cargo test --workspace
	cargo test --workspace --all-features

	cargo clippy --workspace --tests --examples

	cargo doc --no-deps --all-features

	cargo machete
