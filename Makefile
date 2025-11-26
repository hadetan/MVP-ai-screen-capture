SHELL := /bin/bash

.PHONY: deps-fedora deps-debian package-linux build

deps-fedora:
	./scripts/install-deps-fedora.sh

deps-debian:
	./scripts/install-deps-debian.sh

build:
	npm run build

package-linux: build
	./scripts/build-linux-packages.sh
