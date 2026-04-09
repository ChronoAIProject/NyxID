# JWT Signing Keys

The `default.private.pem` and `default.public.pem` files are **quickstart-only
keys** shipped with the repo so you can start NyxID without generating your own.

## IMPORTANT: Replace for production

These keys are public. Anyone who clones this repo can forge JWTs for your
instance if you keep using them. Generate your own before going to production:

```bash
openssl genrsa -out keys/private.pem 4096
openssl rsa -in keys/private.pem -pubout -out keys/public.pem
```

Custom keys (`private.pem` / `public.pem`) are gitignored and will not be
committed. The default keys are tracked intentionally for quickstart convenience.
