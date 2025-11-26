#!/usr/bin/env bash
set -euo pipefail
echo "Installing build dependencies for Fedora (rpm and deb building)..."
sudo dnf install -y rpm-build rpmdevtools redhat-rpm-config gcc-c++ pkgconfig gtk3-devel webkit2gtk3-devel openssl-devel libappindicator gtk3 libnotify
echo "Dependencies installed. You might still need dpkg tools if you plan to build .deb packages on Fedora. Install dpkg by: sudo dnf install -y dpkg" 
