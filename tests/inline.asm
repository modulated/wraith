OUT = $1000
    ; Memory-mapped OUT (read-write)
main:
    ; Call add with 2 args
    LDA #$03
    STA $50
    LDA #$04
    STA $51
    JSR add
    STA $1000
    RTS
add:
    LDA #$00
    STA $42
    CLC
    LDA $40
    ADC $41
    STA $42
    LDA $42
    RTS
