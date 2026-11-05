// Verify the signature with the public key extracted from the packaged app's Info.plist.
import CryptoKit
import Foundation

let args = CommandLine.arguments
guard args.count == 4, let rawKey = Data(base64Encoded: args[1]),
      let signature = Data(base64Encoded: args[2]) else { exit(2) }
let key = try Curve25519.Signing.PublicKey(rawRepresentation: rawKey)
let archive = try Data(contentsOf: URL(fileURLWithPath: args[3]), options: .mappedIfSafe)
guard key.isValidSignature(signature, for: archive) else {
    FileHandle.standardError.write(Data("Sparkle signing key does not match the public key in the app.\n".utf8))
    exit(1)
}
