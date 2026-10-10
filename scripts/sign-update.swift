// Ed25519 signing for update checksums. Run with `swift scripts/sign-update.swift`.
//
//   sign-update.swift --generate           print a new key pair (private, then public; base64 raw)
//   UPDATE_SIGNING_KEY=<private> sign-update.swift dist/SHA256SUMS > dist/SHA256SUMS.sig
//
// The private key lives only in the UPDATE_SIGNING_KEY repository secret (and your
// own backup). The public key is embedded in the app (UpdateConfig.publicKeys).
// To rotate keys, see docs/RELEASING.md.

import CryptoKit
import Foundation

let args = CommandLine.arguments.dropFirst()
if args.first == "--generate" {
    let key = Curve25519.Signing.PrivateKey()
    print("private: \(key.rawRepresentation.base64EncodedString())")
    print("public:  \(key.publicKey.rawRepresentation.base64EncodedString())")
    exit(0)
}
guard let path = args.first else {
    FileHandle.standardError.write("usage: sign-update.swift <file> (key in $UPDATE_SIGNING_KEY) | --generate\n".data(using: .utf8)!)
    exit(2)
}
guard let b64 = ProcessInfo.processInfo.environment["UPDATE_SIGNING_KEY"], !b64.isEmpty,
    let raw = Data(base64Encoded: b64.trimmingCharacters(in: .whitespacesAndNewlines)),
    let key = try? Curve25519.Signing.PrivateKey(rawRepresentation: raw)
else {
    FileHandle.standardError.write("UPDATE_SIGNING_KEY is missing or invalid\n".data(using: .utf8)!)
    exit(1)
}
let data = try Data(contentsOf: URL(fileURLWithPath: path))
let signature = try key.signature(for: data)
print(signature.base64EncodedString())
