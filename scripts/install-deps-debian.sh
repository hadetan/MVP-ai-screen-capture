#!/usr/bin/env bash
set -euo pipefail
echo "Installing build dependencies for Debian/Ubuntu (deb packaging)..."
sudo apt update
sudo apt install -y build-essential fakeroot dpkg-dev debhelper pkg-config libgtk-3-dev libwebkit2gtk-4.0-dev libssl-dev
echo "Dependencies installed. For building RPMs, consider using a Fedora/ CentOS environment or installing rpm tools: sudo apt install -y rpm" 
