PKGDIR := packaging/arch
ACT := act
ACT_CACHE := /tmp/act-cache
ACT_ARTIFACTS := /tmp/act-artifacts
ACT_ARCH := linux/amd64
ACT_RUNNER_IMAGE := catthehacker/ubuntu:act-latest
ACT_FLAGS := --container-architecture $(ACT_ARCH) -P ubuntu-latest=$(ACT_RUNNER_IMAGE) --artifact-server-path $(ACT_ARTIFACTS)

.PHONY: help fmt check test run ci-local ci-local-rust ci-local-arch arch-srcinfo arch-srcinfo-git arch-build-release arch-build-git arch-build-git-local

help:
	@echo "Common targets:"
	@echo "  make fmt                 - Run rustfmt"
	@echo "  make check               - Run cargo check"
	@echo "  make test                - Run cargo test"
	@echo "  make run                 - Run hyprfinity (cargo run --)"
	@echo "  make ci-local            - Run local GitHub Actions CI jobs via act"
	@echo "  make ci-local-rust       - Run rust workflow job via act"
	@echo "  make ci-local-arch       - Run arch-package-git workflow job via act"
	@echo "  make arch-srcinfo        - Print srcinfo for release PKGBUILD"
	@echo "  make arch-srcinfo-git    - Print srcinfo for git PKGBUILD"
	@echo "  make arch-build-release  - Build/install release Arch package"
	@echo "  make arch-build-git      - Build/install git Arch package (remote source)"
	@echo "  make arch-build-git-local- Build/install git Arch package from local repo"

fmt:
	cargo fmt

check:
	cargo check

test:
	cargo test

run:
	cargo run --

ci-local: ci-local-rust ci-local-arch

ci-local-rust:
	mkdir -p "$(ACT_CACHE)" "$(ACT_ARTIFACTS)"
	XDG_CACHE_HOME="$(ACT_CACHE)" $(ACT) push -j rust $(ACT_FLAGS)

ci-local-arch:
	mkdir -p "$(ACT_CACHE)" "$(ACT_ARTIFACTS)"
	XDG_CACHE_HOME="$(ACT_CACHE)" $(ACT) push -j arch-package-git $(ACT_FLAGS)

arch-srcinfo:
	cd $(PKGDIR) && makepkg --printsrcinfo -p PKGBUILD

arch-srcinfo-git:
	cd $(PKGDIR) && makepkg --printsrcinfo -p PKGBUILD-git

arch-build-release:
	cd $(PKGDIR) && makepkg -p PKGBUILD -si

arch-build-git:
	cd $(PKGDIR) && HYPRFINITY_GIT_SOURCE=remote makepkg -p PKGBUILD-git -si

arch-build-git-local:
	cd $(PKGDIR) && HYPRFINITY_GIT_SOURCE=local makepkg -p PKGBUILD-git -si
