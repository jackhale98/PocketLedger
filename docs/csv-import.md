# CSV Import Guide

PocketHLedger can import transactions from bank CSV exports using
hledger-compatible rules files. This lets you automate categorization
of transactions from any bank.

## Quick Start

1. Export a CSV from your bank (Settings > Download Statements)
2. Create a `.csv.rules` file that maps the CSV columns
3. In the app: **Settings > Tools > Import CSV**
4. Pick the CSV file and the rules file
5. Preview the transactions, deselect any duplicates
6. Tap **Import**

## CSV Rules File Format

A rules file is a plain text file (typically named `bankname.csv.rules`)
that tells PocketHLedger how to interpret your CSV.

### Minimal example

```
skip 1
fields date, description, amount
date-format %m/%d/%Y
currency $
account1 assets:checking
```

### Full example

```
# Chase credit card statement rules
skip 1
separator ,
fields date, , description, , amount
date-format %m/%d/%Y
currency $
account1 liabilities:chase

# Categorize by merchant name
if WHOLE FOODS
TRADER JOE
STOP & SHOP
  account2 expenses:groceries

if SHELL OIL
SUNOCO
EXXON
  account2 expenses:transport:fuel

if NETFLIX
SPOTIFY
HULU
  account2 expenses:subscriptions

if UBER
LYFT
  account2 expenses:transport:rideshare

if CHIPOTLE
STARBUCKS
DOORDASH
  account2 expenses:dining

# Everything else goes to uncategorized
account2 expenses:uncategorized
```

## Directive Reference

### `skip N`

Skip the first N rows (usually 1 for the header). Default: 1.

```
skip 1
```

### `fields`

Comma-separated list of field names mapping to CSV columns, left to right.
Use an empty name to skip a column.

```
fields date, , description, , amount
```

Recognized field names:
- `date` - transaction date (required)
- `description` - payee/description
- `amount` - transaction amount (positive = income, negative = expense)
- `amount-in` - credit/deposit column (always positive)
- `amount-out` - debit/payment column (always positive, will be negated)
- `balance` - running balance (not used for import, but reserved)
- `comment` - transaction note
- `code` - transaction code/reference

### `date-format`

strftime format for parsing the date column. Default: `%Y-%m-%d`.

Common formats:
```
date-format %m/%d/%Y     # 03/15/2026
date-format %d/%m/%Y     # 15/03/2026
date-format %Y-%m-%d     # 2026-03-15
date-format %m-%d-%Y     # 03-15-2026
```

### `currency`

Commodity symbol to add to amounts (when the CSV has bare numbers).

```
currency $
currency EUR
```

### `separator`

Field separator character. Default: `,` (comma).

```
separator ;        # Semicolons (common in European exports)
separator \t       # Tab-separated
```

### `decimal-mark`

Decimal point character. Default: `.` (period).

```
decimal-mark ,     # European: 1.234,56 means 1234.56
```

When set to `,`, periods are treated as thousands separators.

### `newest-first`

If your CSV has the newest transactions first (most bank exports do),
add this to reverse them into chronological order:

```
newest-first
```

### `account1`

The account this CSV file represents (your bank account).

```
account1 assets:checking
account1 liabilities:chase
```

### `account2`

The default counterpart account. Can be overridden by `if` blocks.

```
account2 expenses:uncategorized
```

### `if` blocks

Conditional rules that match CSV rows using regular expressions.
When a pattern matches, the assignments below it are applied.

Single pattern:
```
if WHOLE FOODS
  account2 expenses:groceries
```

Multiple patterns (OR logic):
```
if
UBER
LYFT
TAXI
  account2 expenses:transport
```

Patterns are case-insensitive and matched against the entire CSV row.
Regex syntax is supported:

```
if (WHOLE|TRADER).*(FOODS|JOE)
  account2 expenses:groceries
```

You can override any field in an if block:
```
if PAYROLL
  account2 income:salary
  comment automated payroll deposit
```

### Field substitution

Use `%fieldname` or `%N` (1-based column index) in assignments:

```
description %2 - %3    # Combine columns 2 and 3
comment Ref: %code      # Use the "code" field
```

## amount-in / amount-out

Some banks provide separate columns for debits and credits instead
of a single signed amount:

```csv
Date,Description,Credit,Debit,Balance
03/15/2026,DEPOSIT,500.00,,5500.00
03/16/2026,GROCERY,,87.42,5412.58
```

Map them with:
```
fields date, description, amount-in, amount-out, balance
```

`amount-in` values become positive (income/deposit).
`amount-out` values become negative (expense/payment).

## Tips

- **Start simple**: begin with just `skip`, `fields`, `date-format`, `currency`,
  and `account1`. Review the import preview and add `if` blocks to fix
  miscategorized transactions.

- **One rules file per bank**: keep `chase.csv.rules`, `bofa.csv.rules`, etc.
  alongside your journal. The rules file builds up over time as you add
  more categorization patterns.

- **Review before importing**: the preview step shows all transactions with
  checkboxes. Deselect any that are already in your journal to avoid
  duplicates.

- **Imported transactions are marked cleared** (`*`) so you can distinguish
  them from manually entered ones.

## Compatibility

The rules format is compatible with hledger's CSV rules. Rules files
written for `hledger import` should work in PocketHLedger. Advanced
features like `balance-type`, `if %field`, and multi-line `if` tables
are not yet supported.
