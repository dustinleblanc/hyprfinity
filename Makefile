PKGDIR := packaging/arch

.PHONY: help fmt check test run arch-srcinfo arch-srcinfo-git arch-build-release arch-build-git arch-build-git-local

help:
	@echo "Common targets:"
	@echo "  make fmt                 - Run rustfmt"
	@echo "  make check               - Run cargo check"
	@echo "  make test                - Run cargo test"
	@echo "  make run                 - Run hyprfinity (cargo run --)"
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
