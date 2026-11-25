#!/bin/sh
# Print release notes for a tag, listing every commit since the previous one.
set -eu
tag=$1
previous=$(git describe --tags --abbrev=0 --match 'v*' "$tag^" 2>/dev/null || true)
if [ -n "$previous" ]; then
    range="$previous..$tag"
    echo "## Changes since $previous"
else
    range=$tag
    echo "## Changes"
fi
echo
# Each commit is a heading line followed by its indented description.
git log --reverse --format='- **%s** (%h)%n%w(0,2,2)%b%w(0,0,0)' "$range" | sed '/^[[:space:]]*$/d'
echo
echo "---"
echo "The apps are not code signed or notarised, so macOS and Windows warn on first launch."
echo "See the README for setup and for which platforms have been tested by hand."
