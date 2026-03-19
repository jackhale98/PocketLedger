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

- Rust stable with `aarch64-apple-ios` target (for iOS)
- Node.js 20+
- Tauri CLI (`npm install @tauri-apps/cli`)

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
npx tauri android build                      # Android
```

## CI/CD

GitHub Actions workflows in `.github/workflows/`:

| Workflow | Trigger | What it does |
|----------|---------|-------------|
| `ci.yml` | Push to main | Rust tests, cargo check, TypeScript check, Vite build |
| `build-ios.yml` | Tag `v*` | Build IPA, sign, upload to TestFlight |
| `build-android.yml` | Tag `v*` | Build debug APK |
| `build-desktop.yml` | Tag `v*` | Build for macOS, Linux, Windows |

See [docs/ios-testflight-setup.md](docs/ios-testflight-setup.md) for Apple signing setup.

## Testing

```bash
cargo test --workspace   # 129 Rust tests
npx tsc --noEmit         # TypeScript type check
npx vite build           # Frontend build
```

## License

MIT
