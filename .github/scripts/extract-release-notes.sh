#!/usr/bin/env bash
set -euo pipefail

tag="${1:?tag is required}"
version="${tag#v}"

awk -v heading="## [${version}]" '
  /^## \[/ {
    if (found) {
      exit
    }
    if (index($0, heading) == 1) {
      found = 1
      next
    }
  }
  found {
    print
  }
  END {
    if (!found) {
      exit 1
    }
  }
' CHANGELOG.md
