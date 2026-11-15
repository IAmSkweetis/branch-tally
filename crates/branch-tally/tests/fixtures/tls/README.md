# Loopback TLS fixtures

Test-only. A throwaway CA and one server leaf (`localhost`, `127.0.0.1`, `::1`), valid for 100
years, so the `prices update` tests can run a real TLS server on loopback (#98). The shipped binary
trusts only its bundled roots and has no hook to add one: tests add `ca.der` to a ureq agent
through `ureq::tls::RootCerts`.

| File | Holds |
| ---- | ----- |
| `ca.der` | The CA certificate (DER). The root the test client trusts. |
| `leaf.der` | The server certificate (DER), signed by the CA. |
| `leaf.key.der` | The server's private key (PKCS#8 DER). Worthless: it protects nothing. |

The CA's own key was discarded after signing. To regenerate (this replaces all three files), make
an ECDSA P-256 CA and sign a P-256 leaf with it:

```sh
openssl ecparam -name prime256v1 -genkey -noout -out ca.key
openssl req -x509 -new -key ca.key -sha256 -days 36500 -subj "/CN=branch-tally test CA" \
  -addext "basicConstraints=critical,CA:TRUE,pathlen:0" \
  -addext "keyUsage=critical,keyCertSign,cRLSign" -out ca.pem
openssl ecparam -name prime256v1 -genkey -noout -out leaf.ec.key
openssl pkcs8 -topk8 -nocrypt -in leaf.ec.key -outform DER -out leaf.key.der
openssl req -new -key leaf.ec.key -subj "/CN=localhost" -out leaf.csr
# leaf.ext: basicConstraints=critical,CA:FALSE / keyUsage=critical,digitalSignature /
#           extendedKeyUsage=serverAuth / subjectAltName=DNS:localhost,IP:127.0.0.1,IP:::1
openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 36500 -sha256 \
  -extfile leaf.ext -out leaf.pem
openssl x509 -in ca.pem -outform DER -out ca.der
openssl x509 -in leaf.pem -outform DER -out leaf.der
```

The files are DER on purpose: there is no PEM `PRIVATE KEY` block for a secret scanner to flag.
