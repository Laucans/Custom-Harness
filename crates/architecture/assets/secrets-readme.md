# Secrets

Every secret a port needs (`ports.json`, `secrets`) lives here, encrypted with
[sops](https://github.com/getsops/sops) and age, one file per environment:
`secrets/<env>.enc.yaml`. Only names reach the agents; the values are a
human's to provide — one `harness:human` task per port asks for them.

    sops secrets/production.enc.yaml      # edit, decrypted in your editor only

Never commit a decrypted file: the gate `secrets-encrypted` refuses it.
The tests, the CI and the mock-up never need these values: they run on each
port's fake adapter, and Postgres in Docker.
