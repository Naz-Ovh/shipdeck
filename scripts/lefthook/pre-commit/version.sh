#!/bin/sh
# Every version the staged README, examples and schema name must be the one
# in Cargo.toml's [workspace.package].
version=$(git show :Cargo.toml | sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"/\1/p')
status=0
for file in README.md deploy.schema.json $(git ls-files 'examples/*/deploy.yaml'); do
  for found in $(git show ":$file" |
    grep -oE 'shipdeck/v[^/]+/deploy\.schema\.json|git tag v[^ ]+' |
    sed -E 's#^shipdeck/v([^/]+)/.*#\1#; s#^git tag v##' | sort -u); do
    if [ "$found" != "$version" ]; then
      echo "$file names v$found, but Cargo.toml says $version" >&2
      status=1
    fi
  done
done
exit $status
