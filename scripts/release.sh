#!/bin/sh
# Release Prometeu.
#
# sh scripts/release.sh          Calculate the version from commits.
# sh scripts/release.sh 0.2.0    Select a version explicitly.
# sh scripts/release.sh publish Publish the CI-created draft.
# sh scripts/release.sh draft VERSION DIRECTORY Upload CI-verified packages to a draft.
#
# This script controls version, changelog and tag. CI builds and signs a draft release in this
# repository. Install and review its macOS DMG and Linux AppImage before publishing.
# The updater accepts only newer versions; publishing a bad release requires another release to recover.
# Conventional Commits supply git-cliff, CHANGELOG.md and release notes. Features bump minor; fixes
# bump patch. While versions remain 0.x, breaking changes also bump minor; see cliff.toml.
# Signing keys stay in ~/.tauri/prometeu.key, with the password in Keychain and CI copies in repository
# Secrets. Losing both copies prevents updating existing installations; retain a secure backup.
set -eu
cd "$(dirname "$0")/.."

REPO=prometeucorp/prometeu

die() { echo "$*" >&2; exit 1; }
# Call git-cliff directly so npm does not consume its flags as npm configuration.
cliff() {
  [ -x node_modules/.bin/git-cliff ] || die "git-cliff não está instalado — rode npm install"
  node_modules/.bin/git-cliff "$@"
}

# Cut a release.

cut() {
  VERSION=${1:-}

  [ -z "$(git status --porcelain)" ] || die "há mudança não commitada — resolva antes de soltar"
  [ "$(git rev-parse --abbrev-ref HEAD)" = main ] || die "release sai da main"
  git fetch -q --tags origin main
  [ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] \
    || die "a main local não é a origin/main — dê pull (ou push) antes"

  PREV=$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || echo "")
  if [ -n "$PREV" ] && [ "$(git rev-list --count "$PREV..HEAD")" = 0 ]; then
    die "nada desde $PREV — não há o que soltar"
  fi

  [ -n "$VERSION" ] || VERSION=$(cliff --bumped-version | sed 's/^v//')
  case "$VERSION" in
    *.*.*) ;;
    *) die "versão inválida: '$VERSION' (ex.: 0.1.8)" ;;
  esac
  ! git rev-parse -q --verify "refs/tags/v$VERSION" >/dev/null || die "a tag v$VERSION já existe"

  # Commits without user-visible changes do not justify a release. Observable changes need an appropriate
  # feat/fix/perf entry.
  NOTES=$(cliff --unreleased --tag "v$VERSION" --strip all)
  printf '%s\n' "$NOTES" | grep -q '^- ' \
    || die "nenhum feat, fix ou perf desde ${PREV:-o começo} — nada para contar na $VERSION"

  echo "== $VERSION  (anterior: ${PREV:-nenhuma})"
  echo
  printf '%s\n' "$NOTES"

  # npm updates package.json and package-lock.json together. Update Tauri metadata and Cargo.lock
  # explicitly to avoid dirtying the next build.
  npm version "$VERSION" --no-git-tag-version --allow-same-version >/dev/null
  python3 - "$VERSION" <<'PY'
import json, pathlib, re, sys
v = sys.argv[1]

p = pathlib.Path("src-tauri/tauri.conf.json")
c = json.loads(p.read_text()); c["version"] = v
p.write_text(json.dumps(c, indent=2) + "\n")

p = pathlib.Path("src-tauri/Cargo.toml")
p.write_text(re.sub(r'(?m)^version = "[^"]+"', f'version = "{v}"', p.read_text(), count=1))

p = pathlib.Path("src-tauri/Cargo.lock")
p.write_text(re.sub(r'(name = "prometeu"\nversion = )"[^"]+"', rf'\1"{v}"', p.read_text(), count=1))

p = pathlib.Path("packaging/arch/PKGBUILD")
text = re.sub(r"(?m)^pkgver=.*$", f"pkgver={v}", p.read_text(), count=1)
p.write_text(re.sub(r"(?m)^pkgrel=.*$", "pkgrel=1", text, count=1))
PY

  cliff --unreleased --tag "v$VERSION" --prepend CHANGELOG.md
  cat -s CHANGELOG.md > CHANGELOG.md.tmp && mv CHANGELOG.md.tmp CHANGELOG.md

  LOG=$(mktemp -t prometeu-test)
  npm test >"$LOG" 2>&1 || { cat "$LOG"; rm -f "$LOG"; die "testes vermelhos — nada foi commitado"; }
  rm -f "$LOG"

  git add -A
  git commit -qm "chore(release): v$VERSION"
  # Preserve Markdown headings with --cleanup=whitespace; default cleanup removes lines beginning with #.
  git tag -a "v$VERSION" --cleanup=whitespace -m "Prometeu $VERSION" -m "$NOTES"
  git push -q origin main "v$VERSION"

  echo
  echo "v$VERSION empurrada. O CI constrói, assina e deixa uma draft em $REPO."
  watch_run "v$VERSION"
}

# Poll the tag's release workflow without gh run watch because agent sessions may not have an interactive
# terminal.
watch_run() {
  TAG=$1
  RUN=""
  for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do
    RUN=$(gh run list --workflow=release.yml --branch="$TAG" --json databaseId -q '.[0].databaseId' 2>/dev/null || true)
    [ -n "$RUN" ] && break
    sleep 5
  done
  [ -n "$RUN" ] || { echo "não achei o run do release para $TAG — veja em $(gh repo view --json url -q .url)/actions"; return 0; }

  URL=$(gh run view "$RUN" --json url -q .url)
  echo "acompanhando $URL"
  while :; do
    STATUS=$(gh run view "$RUN" --json status,conclusion -q '.status + " " + (.conclusion // "")')
    case "$STATUS" in
      "completed success")
        echo
        echo "draft pronta: https://github.com/$REPO/releases/tag/$TAG"
        echo "install and review the macOS DMG and Linux AppImage. Then: sh scripts/release.sh publish"
        return 0 ;;
      completed*)
        die "o run terminou como '${STATUS#completed }' — leia o log em $URL antes de qualquer coisa" ;;
    esac
    printf '  %s  %s\n' "$(date +%H:%M:%S)" "$STATUS"
    sleep 30
  done
}

# Upload new releases only. Existing drafts may belong to an older commit of a moved tag;
# fail instead of reusing them or replacing assets during concurrent publication.
draft() {
  VERSION=$1
  DIRECTORY=$2
  TAG=v$VERSION
  for want in Prometeu_aarch64.dmg Prometeu_aarch64.app.tar.gz \
              Prometeu_aarch64.app.tar.gz.sig Prometeu_x86_64.AppImage \
              Prometeu_x86_64.AppImage.sig latest.json; do
    [ -f "$DIRECTORY/$want" ] || die "Missing $want"
  done
  [ -n "${RELEASE_NOTES:-}" ] || die "Missing release notes"
  if DRAFT=$(gh release view "$TAG" -R "$REPO" --json isDraft -q .isDraft 2>/dev/null); then
    [ "$DRAFT" = true ] || die "$TAG is already published; refusing to replace assets"
    die "$TAG draft already exists; remove the draft manually before rebuilding"
  fi
  NOTES_FILE=$(mktemp "${TMPDIR:-/tmp}/prometeu-notes.XXXXXX")
  printf '%s\n' "$RELEASE_NOTES" > "$NOTES_FILE"
  trap 'rm -f "$NOTES_FILE"' EXIT
  gh release create "$TAG" -R "$REPO" --verify-tag --draft --title "$VERSION" --notes-file "$NOTES_FILE"
  gh release upload "$TAG" -R "$REPO" \
    "$DIRECTORY/Prometeu_aarch64.dmg" \
    "$DIRECTORY/Prometeu_aarch64.app.tar.gz" \
    "$DIRECTORY/Prometeu_aarch64.app.tar.gz.sig" \
    "$DIRECTORY/Prometeu_x86_64.AppImage" \
    "$DIRECTORY/Prometeu_x86_64.AppImage.sig" \
    "$DIRECTORY/latest.json"
}

# Publish the reviewed draft.

publish() {
  VERSION=${1:-$(node -p "require('./package.json').version")}
  TAG=v$VERSION

  DRAFT=$(gh release view "$TAG" -R "$REPO" --json isDraft -q .isDraft 2>/dev/null) \
    || die "não existe release $TAG em $REPO — o CI terminou?"
  [ "$DRAFT" = true ] || die "$TAG já está publicada"

  ASSETS=$(gh release view "$TAG" -R "$REPO" --json assets -q '.assets[].name')
  # Stable asset names preserve download links for both platforms.
  for want in Prometeu_aarch64.dmg \
              Prometeu_aarch64.app.tar.gz \
              Prometeu_aarch64.app.tar.gz.sig \
              Prometeu_x86_64.AppImage \
              Prometeu_x86_64.AppImage.sig \
              latest.json; do
    printf '%s\n' "$ASSETS" | grep -qx "$want" || die "falta $want na draft — o CI terminou inteiro?"
  done

  CONCL=$(gh run list --workflow=release.yml --branch="$TAG" --json conclusion -q '.[0].conclusion // "?"')
  [ "$CONCL" = success ] || die "o run do release para $TAG está '$CONCL' — publique só com verde"

  gh release edit "$TAG" -R "$REPO" --draft=false --latest
  echo
  echo "$VERSION published. macOS and Linux AppImage installs check at startup and every hour."
  echo "Linux packages installed through a package manager must be updated through that manager."
}

case "${1:-}" in
  draft) shift; draft "$@" ;;
  publish) shift; publish "$@" ;;
  -h|--help|help) sed -n '2,6p' "$0"; exit 0 ;;
  *) cut "$@" ;;
esac
