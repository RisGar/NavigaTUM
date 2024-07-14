#!/bin/bash

sudo rm -rf $(dirname "$0")/fonts $(dirname "$0")/sprites

# fonts
wget -q -O /tmp/roboto-android.zip https://github.com/googlefonts/roboto/releases/download/v2.138/roboto-android.zip
unzip /tmp/roboto-android.zip -d $(dirname "$0")/fonts/
rm /tmp/roboto-android.zip

# sprites
wget -q -O /tmp/sprites_maki.zip https://github.com/mapbox/maki/zipball/main
unzip -q /tmp/sprites_maki.zip -d /tmp/sprites/
rm /tmp/sprites_maki.zip
mkdir $(dirname "$0")/sprites/
mv /tmp/sprites/mapbox-maki-*/icons/* $(dirname "$0")/sprites/
rm -fr /tmp/sprites/
