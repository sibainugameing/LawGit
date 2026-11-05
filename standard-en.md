# lawgit Legal Document Data Specification
**Version 1.1**

## 1. Purpose

This specification defines the data format, identifiers, file structure, and Git-based management rules for legal documents in lawgit.

Each legal document is assigned a UUID as its primary identifier. The e-Gov Legal ID is stored as an external identifier.

## 2. General Principles

1. Each legal document MUST be stored in a separate XML file.
2. Each legal document MUST have a unique lawgit UUID.
3. The e-Gov Legal ID MUST be stored as an external identifier when available.
4. Legal names and official legal numbers MUST NOT be used as primary identifiers.
5. Changes to legal documents MUST be tracked using Git.
6. A UUID MUST NOT be changed or reused once assigned.

## 3. Identifiers

### 3.1 lawgit UUID

A UUID version 4 MUST be used to uniquely identify each legal document.

Example:

`550e8400-e29b-41d4-a716-446655440000`

Rules:

- A UUID MUST be generated when a legal document is registered.
- A UUID MUST remain unchanged when the legal document is amended, renamed, or moved.
- UUIDs MUST be unique within the repository.
- UUIDs belonging to deleted legal documents MUST NOT be reused.
- The UUID MUST be stored in the XML document.

### 3.2 e-Gov Legal ID

The e-Gov Legal ID MUST be stored when available.

Example:

`415AC0000000057`

Rules:

- The e-Gov Legal ID MUST be stored as a string.
- It MUST NOT replace the lawgit UUID.
- lawgit MUST NOT generate e-Gov Legal IDs.
- The element MAY be omitted for legal documents that do not have an e-Gov Legal ID.

### 3.3 Official Legal Number

The official legal number assigned upon promulgation MAY be stored.

Example:

`明治二十九年法律第八十九号`

The official legal number MUST be treated as metadata, not as a primary identifier.

## 4. XML File Format

### 4.1 General Requirements

| Property | Requirement |
|---|---|
| XML Version | XML 1.0 |
| Encoding | UTF-8 |
| File Extension | `.xml` |
| Line Ending | LF |
| Root Element | `law` |

### 4.2 Standard Structure

```xml
<?xml version="1.0" encoding="UTF-8"?>
<law>
    <identifiers>
        <uuid>550e8400-e29b-41d4-a716-446655440000</uuid>
        <e-gov-id>415AC0000000057</e-gov-id>
    </identifiers>

    <metadata>
        <name>Legal Document Name</name>
        <law-number>Official Legal Number</law-number>
        <type>Act</type>
        <promulgated-date>2026-01-15</promulgated-date>
        <effective-date>2026-04-01</effective-date>
    </metadata>

    <body>
        <!-- Legal text -->
    </body>
</law>
```

## 5. Element Definitions

### 5.1 `law`

The root element representing a legal document.

The following child elements MUST be present:

- `identifiers`
- `metadata`
- `body`

### 5.2 `identifiers`

Contains identifiers associated with the legal document.

| Element | Requirement | Description |
|---|---|---|
| `uuid` | Required | lawgit UUID |
| `e-gov-id` | Optional | e-Gov Legal ID |

### 5.3 `metadata`

Contains basic information about the legal document.

| Element | Requirement | Description |
|---|---|---|
| `name` | Required | Legal document name |
| `law-number` | Optional | Official legal number |
| `type` | Required | Legal document type |
| `promulgated-date` | Optional | Promulgation date of this legal instrument (YYYY-MM-DD) |
| `effective-date` | Optional | Commencement date of this legal instrument (YYYY-MM-DD) |

The `type` element MUST identify the type of legal document.

Examples include:

- Act
- Cabinet Order
- Imperial Ordinance
- Ministerial Ordinance
- Regulation
- Local Ordinance
- Other

### 5.4 `body`

Contains the legal text.

The legal text MUST be represented using XML structural elements. Articles, paragraphs, and items MUST NOT be stored as a single unstructured text string.

## 6. File Organization

Legal documents MUST be stored using their UUID as the filename.

```text
repository/
├── .git/
├── laws/
│   ├── 550e8400-e29b-41d4-a716-446655440000.xml
│   └── 7c9e6679-7425-40de-944b-e07fc1f90ae7.xml
└── README.md
```

Rules:

- The filename MUST match the UUID contained in the XML document.
- Legal document names MUST NOT be used as filenames.
- A UUID MUST NOT be changed as a result of renaming or moving a file.
- Each XML file MUST contain exactly one legal document.

## 7. Git Management

Changes to legal documents MUST be recorded using Git commits.

| Git Feature | lawgit Usage |
|---|---|
| Commit | Legal document changes and amendments |
| Commit Message | Description of changes |
| Branch | Proposed amendments and working versions |
| Diff | Comparison of legal text |
| Merge | Integration of proposed amendments |
| Tag | Identification of a specific repository state |

The lawgit UUID MUST remain independent of Git commit hashes.

### 7.1 Amendment Commit Dates

For an amendment commit, the promulgation date of the amending law MUST be used for both the Git author date and committer date. The time is set to 00:00 Japan Standard Time.

The commencement date(s) of the amendment MUST be recorded as commit-message trailers.

```text
Partial Amendment to the Road Traffic Act

LawGit-Amendment-Promulgated-Date: 2026-11-05
LawGit-Effective-Date: 2026-12-01
```

When commencement is staggered, repeat `LawGit-Effective-Date` once for each date. This keeps the legal instrument's original dates separate from the dates of subsequent amendments.

## 8. Data Integrity

The following conditions MUST be validated when registering a legal document:

- The XML document MUST be well-formed.
- The UUID MUST conform to the UUID v4 format.
- The UUID MUST be unique within the repository.
- The filename MUST match the UUID.
- All required elements MUST be present.
- If an e-Gov Legal ID is present, it MUST NOT be empty.
