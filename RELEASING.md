# Releasing

Releases are tag-driven: bump the workspace version, tag, push.

```sh
# 1. bump [workspace.package] version in Cargo.toml, commit
# 2. tag and push
git tag v0.1.0
git push --tags
```

The `Release` workflow builds the `converge` CLI for every supported
target, writes a `SHA256SUMS` manifest, signs it with minisign, and
publishes a GitHub Release. `install.sh` and `converge update` consume
that layout.

## One-time: the signing key

```sh
minisign -G -W -p minisign.pub -s minisign.key
```

- `-W` generates the secret **unencrypted**: CI signs unattended, so the
  key's protection *is* the repository secret store. Anyone who can write
  repo secrets can sign — acceptable for now; move to offline signing if
  the project's threat model grows.
- Put the content of `minisign.key` into the `MINISIGN_SECRET_KEY`
  repository secret, then delete the local file.
- Commit `minisign.pub`, and bake its key line into `install.sh`
  (replace `__MINISIGN_PUBKEY__`) and — from D3 on — the CLI's embedded
  verification key.

Rotation: generate the new pair, ship one release signed by the **old**
key whose binary embeds the **new** public key, then switch the secret.
Installed clients verify that release with the old key and trust the new
one afterwards; `install.sh` picks up the new baked-in key from `main`.

## Version discipline

The workspace version is the wire-compatibility signal: the CLI compares
it against the server's `healthz` and nudges on any mismatch (pre-1.0,
inequality = skew). Tag versions must match the workspace version.
