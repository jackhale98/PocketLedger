# iOS TestFlight Setup (without a Mac)

This guide covers generating Apple signing credentials on Linux, configuring
GitHub secrets, and letting the CI workflow build, sign, and upload PocketHLedger
to TestFlight automatically.

## Prerequisites

- Apple Developer Program membership ($99/year) — https://developer.apple.com/programs/
- OpenSSL installed (`sudo apt install openssl` on Ubuntu)

## Step 1: Generate a Distribution Certificate

```bash
# Generate a private key
openssl genrsa -out ios_dist.key 2048

# Create a Certificate Signing Request (CSR)
openssl req -new -key ios_dist.key -out ios_dist.csr \
  -subj "/CN=Your Name/emailAddress=your@email.com"
```

## Step 2: Upload CSR to Apple Developer Portal

1. Go to https://developer.apple.com/account/resources/certificates/add
2. Select **Apple Distribution**
3. Upload `ios_dist.csr`
4. Download the resulting `distribution.cer`

## Step 3: Create a .p12 Bundle

**Important:** Use the `-legacy` flag so the .p12 is compatible with the macOS
`security` framework on GitHub Actions runners.

```bash
# Convert Apple's .cer (DER format) to PEM
openssl x509 -inform DER -in distribution.cer -out distribution.pem

# Bundle into .p12 — note the -legacy flag!
openssl pkcs12 -export -legacy -out distribution.p12 \
  -inkey ios_dist.key -in distribution.pem \
  -password pass:YOUR_P12_PASSWORD
```

## Step 4: Register an App ID

1. Go to https://developer.apple.com/account/resources/identifiers/add/bundleId
2. Select **App IDs** → **App**
3. Description: `PocketHLedger`
4. Bundle ID (Explicit): `com.pockethledger.app`
   - Must match the `identifier` field in `src-tauri/tauri.conf.json`
5. Click **Register**

## Step 5: Create a Provisioning Profile

1. Go to https://developer.apple.com/account/resources/profiles/add
2. Select **App Store Connect** (this covers TestFlight)
3. Select the App ID you just created
4. Select the distribution certificate from Step 2
5. Name it `PocketHLedger Distribution`
6. Download the `.mobileprovision` file

## Step 6: Add GitHub Secrets

1. Go to your GitHub repo → Settings → Environments
2. Create an environment called `ios-release`
3. Add these secrets:

```bash
# Generate base64 values on your Linux machine:
base64 -w 0 distribution.p12        # → APPLE_CERTIFICATE
base64 -w 0 profile.mobileprovision  # → APPLE_PROVISIONING_PROFILE
```

| Secret Name | Value |
|------------|-------|
| `APPLE_CERTIFICATE` | base64-encoded contents of `distribution.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | The password you set in Step 3 |
| `APPLE_TEAM_ID` | Your 10-character Team ID (developer.apple.com → Membership Details) |
| `APPLE_PROVISIONING_PROFILE` | base64-encoded contents of the `.mobileprovision` file |

## Step 7: Create the App in App Store Connect

1. Go to https://appstoreconnect.apple.com → My Apps → "+"
2. New App → iOS
3. Name: `PocketHLedger`
4. Bundle ID: select the one from Step 4
5. SKU: `pockethledger`
6. Save — this creates the TestFlight landing page

## Step 8: Create an App Store Connect API Key

1. Go to App Store Connect → Users and Access → Integrations → App Store Connect API
2. Generate a new key with **App Manager** role
3. Download the `.p8` file and note the Key ID and Issuer ID
4. Base64-encode the key: `base64 -w 0 AuthKey_XXXXXXXX.p8`

Add these secrets to the same `ios-release` environment:

| Secret Name | Value |
|------------|-------|
| `APP_STORE_CONNECT_API_KEY` | base64-encoded `.p8` file |
| `APP_STORE_CONNECT_KEY_ID` | the Key ID shown in App Store Connect |
| `APP_STORE_CONNECT_ISSUER_ID` | the Issuer ID shown at the top of the API Keys page |

If the API key secrets are not configured, the workflow still builds the `.ipa`
and uploads it as a GitHub artifact — you can then upload manually using the
Transporter app.

## Step 9: Trigger a Build

Push a new version tag to trigger the iOS build workflow:

```bash
git tag v0.1.0
git push origin v0.1.0
```

Or trigger manually from the Actions tab using **workflow_dispatch**.

## Step 10: Verify the Build

1. Check the GitHub Actions run — the "Upload to TestFlight" step should show
   `UPLOAD SUCCEEDED`
2. Go to App Store Connect → My Apps → PocketHLedger → TestFlight
3. The build appears after Apple processes it (usually 15-30 minutes)
4. Apple may email about export compliance — answer "No" (no custom encryption)
5. Add internal/external testers — they get a TestFlight notification

## Troubleshooting

| Error | Cause | Fix |
|---|---|---|
| `MAC verification failed during PKCS12 import` | .p12 without `-legacy` flag | Recreate with `openssl pkcs12 -export -legacy ...` |
| `future Xcode project file format (77)` | Xcode < 16.3 | Use `runs-on: macos-15` |
| `No profiles for 'com.pockethledger.app' were found` | Bundle ID mismatch | Verify bundle ID matches `tauri.conf.json` `identifier` |
| `Signing requires a development team` | Missing env var on init | Set `APPLE_DEVELOPMENT_TEAM` on both init and build steps |
| `Library does not include required runtime symbols` | Missing mobile entry point | Ensure `#[cfg_attr(mobile, tauri::mobile_entry_point)]` on `run()` |

## Security Notes

- Never commit `.p12`, `.mobileprovision`, `.p8`, or `.key` files to the repo
- Store signing keys securely — they can't be regenerated
- The `.p8` API key never expires — revoke in App Store Connect if compromised
