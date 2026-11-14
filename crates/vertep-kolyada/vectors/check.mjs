// Checks envelope.json with Node's built-in crypto only: no Rust, no dependencies.
//   node crates/vertep-kolyada/vectors/check.mjs
// It recomputes the JCS bytes of the body, verifies the Ed25519 signature against the published
// key, and recomputes the message hash.
import { createHash, createPublicKey, verify } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const dir = dirname(fileURLToPath(import.meta.url));
const v = JSON.parse(readFileSync(join(dir, 'envelope.json'), 'utf8'));

// RFC 8785 for the values these messages hold: object keys sorted by UTF-16 code unit, no
// whitespace, strings and numbers as ECMAScript writes them.
function jcs(x) {
  if (Array.isArray(x)) return '[' + x.map(jcs).join(',') + ']';
  if (x !== null && typeof x === 'object') {
    return '{' + Object.keys(x).sort().map((k) => JSON.stringify(k) + ':' + jcs(x[k])).join(',') + '}';
  }
  return JSON.stringify(x);
}

const b64 = (s) => Buffer.from(s, 'base64url');
let failed = 0;
const check = (name, ok) => {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}`);
  if (!ok) failed += 1;
};

const signing = jcs(v.message.body);
check('JCS bytes of the body', signing === v.signingBytes);

// An Ed25519 public key as SubjectPublicKeyInfo: a fixed prefix and the 32 key bytes.
const spki = Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), b64(v.publicKey)]);
const key = createPublicKey({ key: spki, format: 'der', type: 'spki' });
check('signature over the body', verify(null, Buffer.from(signing), key, b64(v.message.signature)));

const tampered = jcs({ ...v.message.body, seq: v.message.body.seq + 1 });
check('tampered body refused', !verify(null, Buffer.from(tampered), key, b64(v.message.signature)));

check('message hash', createHash('sha256').update(jcs(v.message)).digest('hex') === v.hash);

process.exit(failed === 0 ? 0 : 1);
