# PocketHLedger

Plain text accounting in your pocket. A mobile-first [hledger](https://hledger.org)-compatible journal viewer and editor built with [Tauri](https://tauri.app) and React.

## Features

- **View transactions** - browse, search, and filter by date with newest/oldest sorting
- **Add & edit transactions** - create entries with account autocomplete and automatic balancing
- **Accounts tree** - expandable account hierarchy with type filters (Assets, Expenses, etc.)
- **Multi-currency support** - tracks commodities independently; "Value in" dropdown converts balances to a target currency using market prices from `P` directives
- **Reports dashboard** - net worth, income vs expenses, and expense breakdown charts
- **Financial statements** - Balance Sheet, Income Statement, Cash Flow with date filtering
- **Budget tracking** - reads `~ monthly` periodic transactions and shows budget-vs-actual with progress bars
- **CSV import** - import bank statements using hledger-compatible `.csv.rules` files with regex categorization
- **Account reconciliation** - match transactions against bank statement balances
- **Include directives** - resolves `include` files and glob patterns (e.g. `include *.journal`)
- **Dark mode** - follows system preference or manual toggle
- **Cross-platform** - iOS (TestFlight), Android, macOS, Linux, Windows

## Getting Started

### Open an existing journal

Tap **Open Journal** and select your `.journal`, `.hledger`, or `.ledger` file. If your journal uses `include` directives, grant access to the folder containing all files.

### Create a new journal

Tap **Create New Journal** to set up a new file with default account categories and your preferred currency.

## Architecture

```
crates/
  hledger-parser/     # Journal parser, CSV rules parser, writer
  hledger-core/       # Ledger resolution, reports, budgets, CSV import
src-tauri/            # Tauri app shell and commands
src/                  # React frontend
```

### Parser (`hledger-parser`)

Hand-written line-based parser (not pest/nom) supporting:
- Transactions with postings, costs (`@`, `@@`), lot prices (`{}`), balance assertions
- Directives: `account`, `commodity`, `P` (price), `include`, `alias`, `decimal-mark`
- Periodic transactions (`~ monthly`) and auto-posting rules (`=`)
- CSV rules files (`.csv.rules`)
- Round-trip writer with `patch_journal` for in-place edits

### Core Engine (`hledger-core`)

- Multi-commodity balance resolution with cost-aware transaction balancing
- Price database from `P` directives and transaction costs, with reverse lookups
- Reports: balance, register, balance sheet, income statement, cash flow
- Time series: net worth, account balance, income vs expenses (monthly)
- Budget engine: extracts periodic transactions, computes budget-vs-actual
- CSV import: applies rules to CSV data, produces transactions
- Verified against `hledger` CLI output on a 791-transaction, 9-commodity test file

### Tauri Commands

28+ commands bridging the Rust engine to the frontend: journal CRUD, reports, budgets, CSV import, reconciliation, autocomplete.

## CSV Import

Import bank transactions from CSV files using hledger-compatible rules:

```
# checking.csv.rules
skip 1
fields date, description, amount, balance
date-format %m/%d/%Y
currency $
account1 assets:checking

if WHOLE FOODS
  account2 expenses:groceries

if SALARY
  account2 income:salary

if UBER
LYFT
  account2 expenses:transport
```

See [docs/csv-import.md](docs/csv-import.md) for the full rules reference.

## Budgets

Define spending targets with periodic transactions in your journal:

```
~ monthly
    expenses:rent          $1,500.00
    expenses:groceries       $400.00
    expenses:dining          $200.00
    income
```

The Budget tab shows progress bars comparing actual spending to targets, with a monthly chart.

## Building

### Prerequisites

- Rust 1.98.0 (pinned in `rust-toolchain.toml`; rustup picks it up automatically) with the `aarch64-apple-ios` / `aarch64-linux-android` targets for mobile
- Node.js 20+
- Tauri CLI (`npm install @tauri-apps/cli`)
- Android: JDK 17, Android SDK with NDK r28+ (`NDK_HOME` set) for 16 KB page-size compliance

### Development

```bash
npm install
npm run tauri dev
```

### iOS

```bash
npx tauri ios init
npx tauri ios dev
```

### Production build

```bash
npx tauri build                              # Desktop
npx tauri ios build --export-method app-store-connect  # iOS
npx tauri android build --aab --apk          # Android (release AAB for Play + APK for sideloading)
```

`src-tauri/Info.ios.plist` (file sharing, export compliance, `.journal`
document types) is merged into the generated Info.plist by the Tauri CLI, so
local iOS builds get it without any CI step. Android release builds also get
signing, system-bar insets and a network-free manifest from the patch scripts
in `scripts/ci/` (see [docs/android-release-setup.md](docs/android-release-setup.md)).

## CI/CD

GitHub Actions workflows in `.github/workflows/`:

| Workflow | Trigger | What it does |
|----------|---------|-------------|
| `ci.yml` | Push to main | Rust tests, cargo check, TypeScript check, Vite build |
| `build-ios.yml` | Tag `v*` | Build IPA (Xcode 26.x pinned, privacy manifest, iPhone-only), sign, upload to TestFlight, keep dSYMs |
| `build-android.yml` | Tag `v*` | Build release AAB + APK (arm64, NDK r28, 16 KB pages, no INTERNET permission), signed when the `ANDROID_*` secrets exist |
| `build-desktop.yml` | Tag `v*` | Build dmg / deb + AppImage / nsis for macOS, Linux, Windows |

All workflows pin Rust to the version in `rust-toolchain.toml`.

See [docs/ios-testflight-setup.md](docs/ios-testflight-setup.md) for Apple
signing setup and [docs/android-release-setup.md](docs/android-release-setup.md)
for the Play upload keystore, Play App Signing, the API 36 target and 16 KB
page-size requirements.

## Releasing

The version lives in `package.json`, `src-tauri/Cargo.toml`,
`src-tauri/tauri.conf.json` and `Cargo.lock`. Change all of them at once:

```bash
scripts/bump-version.sh 0.2.21          # edit the files
scripts/bump-version.sh 0.2.21 --tag    # edit, commit "Release 0.2.21", tag v0.2.21
git push origin main v0.2.21            # the tag triggers the three build workflows
```

Each workflow attaches its artifacts to a single draft GitHub release for the
tag. iOS builds get `CFBundleVersion = <version>.<run number>` and Android
builds get `versionCode = <run number>`, so re-running a tag never produces a
duplicate the stores reject.

## Testing

```bash
cargo test --workspace   # 129 Rust tests
npx tsc --noEmit         # TypeScript type check
npx vite build           # Frontend build
```

## License

MIT
