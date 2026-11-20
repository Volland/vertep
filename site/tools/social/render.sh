#!/bin/sh
# Renders the Bluesky avatar and banner for @vertep.org with headless Chrome,
# so the self-hosted fonts and the site's SVG marks are drawn exactly as on the site.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out="$here/../../assets/img/social"
chrome=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
mkdir -p "$out"
shot() { "$chrome" --headless=new --disable-gpu --hide-scrollbars --allow-file-access-from-files \
  --virtual-time-budget=3000 --force-device-scale-factor=1 --window-size="$2" \
  --screenshot="$out/$3" "file://$here/$1" >/dev/null 2>&1; }
shot avatar.html 1000,1000 bluesky-avatar.png
shot banner.html 3000,1000 bluesky-banner.png
echo "wrote $out/bluesky-avatar.png and $out/bluesky-banner.png"
