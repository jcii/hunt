# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`hunt` is a Rust CLI application for job search automation. It tracks job postings, analyzes employers, manages resumes, and integrates with email to fetch job alerts from LinkedIn and Indeed.

## Build and Test Commands

### Building
```bash
# Build for development
cargo build

# Build optimized release binary
cargo build --release

# Run the application
cargo run -- <command>

# Install locally
cargo install --path .
```

### Testing
```bash
# Run all tests
cargo test

# Run tests for a specific module
cargo test db::tests
cargo test email::tests
cargo test ai::tests

# Run a specific test
cargo test test_exact_title_match_same_employer

# Show test output
cargo test -- --nocapture
```

### Linting
```bash
# Check for compilation errors and warnings
cargo check

# Run clippy for additional lints
cargo clippy

# Format code
cargo fmt
```

**Important:** This project treats warnings as errors (configured in `.cargo/config.toml`). All code must compile without warnings.

## High-Level Architecture

### Core Modules

**Database Layer (`db.rs`)**
- SQLite database with schema for jobs, employers, resumes, and Glassdoor reviews
- Handles all database operations and migrations
- Implements sophisticated duplicate detection for jobs using:
  - Exact URL matching
  - Fuzzy title matching (Jaro-Winkler similarity > 0.8)
  - Substring matching for same employer
  - Case-insensitive comparison

**Email Ingestion (`email.rs`)**
- IMAP-based email fetching from Gmail
- Parses LinkedIn and Indeed job alert emails
- Extracts job details from HTML email bodies
- Uses regex and HTML scraping to parse job postings
- Filters navigation artifacts (e.g., "View all jobs", "Search for jobs")
- LinkedIn-specific parsing handles format: "Title             Company · Location"

**AI Integration (`ai.rs`)**
- Anthropic API client for AI-powered features
- Job analysis and keyword extraction
- Resume tailoring suggestions
- Requires `ANTHROPIC_API_KEY` environment variable

**Data Models (`models.rs`)**
- Core structs: `Job`, `Employer`, `BaseResume`, `ResumeVariant`, `GlassdoorReview`
- Employers track: funding info (YC batch, Crunchbase), controversies, ownership data
- Jobs track: status workflow (new → reviewing → applied → rejected/closed)

### Key Design Patterns

**Employer Status System**
- `ok`: Normal employer (can apply)
- `yuck`: Undesirable employer (apply reluctantly) - reduces ranking score by 20
- `never`: Blocked employer (never apply) - reduces ranking score by 100

**Job Deduplication Strategy**
Jobs are considered duplicates if:
1. Same URL (exact match) OR
2. Same employer AND (exact title match OR substring match OR >80% fuzzy match)

**Resume Management**
- Base resumes: Stored templates in various formats (markdown, plain, JSON, LaTeX)
- Resume variants: Job-specific tailored versions linked to base resume + job
- AI-powered tailoring suggestions when `ANTHROPIC_API_KEY` is available

**Email Parsing Approach**
- Prioritizes HTML content over plain text
- Uses multiple CSS selectors as fallbacks for job extraction
- LinkedIn: Looks for `a[href*='linkedin.com/comm/jobs']`
- Indeed: Looks for `a[href*='indeed.com']` with `/viewjob` or `jk=` patterns
- Strips tracking parameters from URLs (removes query strings)

### Database Schema Notes

The SQLite database auto-migrates on `init()`. Key tables:
- `employers`: Company data with research fields (startup info, controversies, ownership)
- `jobs`: Job postings with employer FK, status, pay range, job codes
- `job_snapshots`: Historical versions of job descriptions
- `base_resumes` / `resume_variants`: Resume management
- `glassdoor_reviews`: Employee reviews with sentiment analysis

Job codes are extracted from common patterns:
- "Job ID:", "Req#:", "Requisition ID:", etc.
- LinkedIn URLs: `/job/view/123456` → `linkedin-123456`
- Standalone patterns: `JR12345`, `R12345`

### Ranking Algorithm

Jobs are ranked by score (see `calculate_score` in `db.rs`):
- Base score: 50
- Pay bonus: Up to +30 points based on max salary
- Employer status penalty: -20 (yuck) or -100 (never)
- Status bonus: +10 (reviewing) or +5 (new)

## Development Workflow

### Database Location
- Database stored at: `~/.local/share/hunt/hunt.db` (XDG data directory)
- Initialize with: `cargo run -- init`
- Destroy all data: `cargo run -- destroy --confirm`

### AI Features
Set API key before using AI commands:
```bash
export ANTHROPIC_API_KEY=your-key-here
```

AI-powered commands: `analyze`, `keywords`, `resume tailor`

### Email Integration
Requires Gmail app password stored in file:
```bash
# Default location: ~/.gmail.app_password.txt
cargo run -- email --username your@gmail.com --password-file ~/.gmail.app_password.txt
```

### Testing Duplicate Detection
The deduplication logic has comprehensive tests in `db.rs`. When modifying duplicate detection:
1. Run existing tests: `cargo test db::tests`
2. Add test cases for new scenarios
3. Test with real data using `cargo run -- cleanup --duplicates --dry-run`

## Rust Edition and Toolchain

- Uses Rust Edition 2024
- Configured to treat all warnings as errors (`-D warnings` in `.cargo/config.toml`)
- Requires bundled SQLite via `rusqlite` with `bundled` feature
