#!/usr/bin/env bash
# Install and start the spotify-tracker systemd user service.
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
unit_dir="${HOME}/.config/systemd/user"

mkdir -p "${unit_dir}"
cp "${repo_dir}/systemd/spotify-tracker.service" "${unit_dir}/"
systemctl --user daemon-reload
systemctl --user enable --now spotify-tracker.service
systemctl --user --no-pager status spotify-tracker.service || true
