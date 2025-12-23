# VSCode Syntax Highlighting for Wraith

This directory contains TextMate grammar files for Wraith language syntax highlighting in Visual Studio Code.

## Files

- `wraith.tmLanguage.json` - TextMate grammar definition
- `language-configuration.json` - Language configuration (brackets, comments, etc.)
- `install-vscode.sh` - Automated installation script

## Installation

### Option 1: Automated Installation (Recommended)

Simply run the install script:

```bash
chmod +x install-vscode.sh
./install-vscode.sh
```

Then **restart VSCode** or run "Developer: Reload Window" (Ctrl+Shift+P / Cmd+Shift+P).

Open any `.wr` file and verify "Wraith" appears in the language mode indicator (bottom right corner).

### Option 2: Manual Installation

1. Install `vsce` (VSCode Extension Manager):
```bash
npm install -g @vscode/vsce
```

2. Create an extension directory:
```bash
mkdir wraith-vscode
cd wraith-vscode
```

3. Create `package.json`:
```json
{
  "name": "wraith-language",
  "displayName": "Wraith Language Support",
  "description": "Syntax highlighting for Wraith programming language",
  "version": "0.1.0",
  "engines": {
    "vscode": "^1.60.0"
  },
  "categories": ["Programming Languages"],
  "contributes": {
    "languages": [{
      "id": "wraith",
      "aliases": ["Wraith", "wraith"],
      "extensions": [".wr"],
      "configuration": "./language-configuration.json"
    }],
    "grammars": [{
      "language": "wraith",
      "scopeName": "source.wraith",
      "path": "./wraith.tmLanguage.json"
    }]
  }
}
```

4. Copy the grammar files:
```bash
cp ../wraith.tmLanguage.json .
cp ../language-configuration.json .
```

5. Package the extension:
```bash
vsce package
```

6. Install the generated `.vsix` file:
   - In VSCode: Extensions → `...` menu → Install from VSIX

## Syntax Highlighting Features

### Keywords
- **Control flow**: `if`, `else`, `while`, `for`, `loop`, `break`, `continue`, `return`
- **Declarations**: `fn`, `let`, `static`, `addr`, `struct`, `enum`, `asm`
- **Modifiers**: `mut`, `zp`, `pub`
- **Constants**: `true`, `false`

### Types
- Unsigned: `u8`, `u16`, `u32`
- Signed: `i8`, `i16`, `i32`
- Other: `bool`, `str`

### Literals
- Decimal: `42`, `1_000`
- Hexadecimal: `0xFF`, `0x0400`
- Binary: `0b1010`
- Strings: `"Hello, world!"`
- Booleans: `true`, `false`

### Operators
- Arithmetic: `+`, `-`, `*`, `/`, `%`
- Comparison: `==`, `!=`, `<`, `>`, `<=`, `>=`
- Logical: `&&`, `||`, `!`
- Bitwise: `&`, `|`, `^`, `<<`, `>>`, `~`

### Attributes
- `#[org(0x8000)]`
- `#[inline]`
- `#[interrupt]`
- `#[no_return]`

### Comments
- Line comments: `// comment`

## Example

```wraith
addr SCREEN = 0x0400;

#[org(0x8000)]
fn main() {
    zp u8 x = 42;
    zp u8 y = 37;

    if x > y {
        SCREEN = x;
    } else {
        SCREEN = y;
    }
}

fn add(a: u8, b: u8) -> u8 {
    return a + b;
}
```

## Troubleshooting

If syntax highlighting doesn't work:

1. Reload VSCode: `Ctrl+Shift+P` → "Developer: Reload Window"
2. Check file association: Ensure `*.wr` files are associated with "Wraith"
3. Verify grammar is loaded: Open a `.wr` file and check the language mode in the bottom right

## Theme Compatibility

The grammar uses standard TextMate scopes, so it should work well with most VSCode themes. For best results, use themes that support:
- `keyword.control`
- `storage.type`
- `entity.name.function`
- `constant.numeric`
- `string.quoted`
- `comment.line`
