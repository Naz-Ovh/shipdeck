#!/bin/sh
# A pushed v<version> tag must point at a commit whose Cargo.toml has that
# version: the release workflow refuses the tag otherwise.
status=0
while read -r local_ref local_oid remote_ref remote_oid; do
  case $remote_ref in refs/tags/v*) ;; *) continue ;; esac
  case $local_oid in *[!0]*) ;; *) continue ;; esac # deleting the tag
  tag=${remote_ref#refs/tags/}
  version=$(git show "$local_oid^{commit}:Cargo.toml" | sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"/\1/p')
  if [ "$tag" != "v$version" ]; then
    echo "tag $tag points at a commit whose Cargo.toml says $version." >&2
    echo "Bump the version to ${tag#v}, commit, and tag that commit." >&2
    status=1
  fi
done
exit $status
