# Bettertricks AUR packages

Each directory mirrors one AUR package base:

- `bettertricks`: stable release built from source (tag `v$pkgver`).
- `bettertricks-bin`: the release `.deb` repackaged.
- `bettertricks-git`: latest `main` built from source.

After the GitHub release exists, run `updpkgsums` in `bettertricks` and `bettertricks-bin`,
regenerate `.SRCINFO` with `makepkg --printsrcinfo > .SRCINFO`, validate in a clean chroot, and
push only `PKGBUILD` and `.SRCINFO` to the matching AUR repository.
