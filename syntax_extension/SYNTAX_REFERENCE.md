# Wraith Syntax Highlighting Reference

## Color-Coded Elements

### 🔵 Keywords (Blue/Purple)

```wraith
fn if else while for loop break continue return
addr static struct enum asm
```

### 🟢 Types (Green/Teal)

```wraith
u8 u16 u32 i8 i16 i32 bool str
```

### 🟠 Modifiers (Orange)

```wraith
pub
```

### 🟡 Constants (Yellow/Orange)

```wraith
true false
SCREEN LED BORDER  // UPPERCASE constants
```

### 🔴 Strings (Red/Brown)

```wraith
"Hello, world!"
```

### 🟣 Numbers (Purple/Magenta)

```wraith
42          // decimal
1_000       // underscore separated
0xFF        // hexadecimal
0b1010      // binary
```

### ⚪ Comments (Gray)

```wraith
// This is a comment
```

### 🟢 Functions (Green)

```wraith
fn main() {  // 'main' highlighted as function name
    add(5, 10);  // 'add' highlighted as function call
}
```

### 🔵 Attributes (Blue)

```wraith
#[org(0x8000)]
#[inline]
```

## Example with Highlighting

```wraith
// Memory-mapped registers
addr LED = 0xD020;      // addr, LED, 0xD020 all highlighted
addr SCREEN = 0x0400;   // differently

#[org(0x8000)]          // Attribute syntax
fn main() {             // fn keyword, main function name
    zp u8 x = 42;       // zp modifier, u8 type, number
    mut u16 y = 0xFF;   // mut modifier, u16 type, hex number

    // Control flow keywords
    if x > 10 {         // if keyword, > operator, number
        {
            LED = 1;    // Variable and assignment
        }
    } else {            // else keyword
        {
            for u8 i in 0..100 {  // for, type, in keywords
                {
                    y = y + i;      // Arithmetic
                }
            }
        }
    }

    // Function call
    SCREEN = calculate(x, y);  // CONSTANT, function call
}

fn calculate(a: u8, b: u8) -> u8 {  // Function declaration
    return a * b;       // return keyword, operators
}
```

## Uninstallation

To remove the extension:

```bash
rm -rf ~/.vscode/extensions/wraith-language-0.1.0/
```

Then restart VSCode.
