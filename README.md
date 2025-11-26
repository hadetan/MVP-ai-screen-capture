# Tauri + React

This template should help get you started developing with Tauri and React in Vite.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Building Linux Packages (deb & rpm)

These instructions will help you build Debian (.deb) and RPM (.rpm) packages for this app. On Fedora you can produce both packages, although building .deb files requires `dpkg` tools.

1. Install system dependencies manually (choose the matching script):
	- Fedora:
	```bash
	scripts/install-deps-fedora.sh
	```
	- Debian/Ubuntu:
	```bash
	scripts/install-deps-debian.sh
	```

2. Build packages:
	```bash
	scripts/build-linux-packages.sh
	```

The produced packages will be located in `src-tauri/target/release/bundle/` by default.

Note: On Fedora, building a `.deb` file may require installing `dpkg` toolchain (e.g., `sudo dnf install -y dpkg`).

If you run into an error when installing the RPM due to a dependency name mismatch like `nothing provides webkit2gtk-4.1 needed by tauri-app`, there's a common naming mismatch between the RPM `Requires` field and Fedora's package names. To fix that automatically, the build script will try to invoke `rpmrebuild` to remove the problematic textual requires.

If the automated fix didn't run, you can run the helper script manually:

```bash
# Rebuild the RPM to remove the problematic textual require and keep only library provides.
sudo dnf install -y rpmrebuild
./scripts/fix-rpm-requires.sh src-tauri/target/release/bundle/rpm/tauri-app-0.1.0-1.x86_64.rpm
sudo dnf install /home/asus/rpmbuild/RPMS/x86_64/tauri-app-0.1.0-1.x86_64.rpm
```

This removes the `webkit2gtk-4.1` textual requirement and leaves the `libwebkit2gtk-4.1.so.0` requirement which your system knows how to satisfy (`webkit2gtk4.1` package provides it on Fedora).

You can also use the Makefile targets:

```bash
make deps-fedora   # Install Fedora build deps
make deps-debian   # Install Debian/Ubuntu build deps
make package-linux # Build packages for Linux
```
