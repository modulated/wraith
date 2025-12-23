SCREEN = $0400
    * = $9000
    ; Function: add
    ;   Params: a: u8, b: u8
    ;   Returns: u8
    ;   Location: $9000
    ;   Attributes: inline
add:
    LDA $41
    STA $20
    LDA $40
    CLC
    ADC $20
    RTS
    * = $9100
    ; Function: multiply
    ;   Params: x: u8, y: u8
    ;   Returns: u8
    ;   Location: $9100
multiply:
    LDA $43
    STA $20
    LDA $42
    TAX
    LDA #$00
    STA $22
    LDA $20
    CMP #$00
    BEQ mul_end_2
    TAY
mul_loop_1:
    TXA
    CLC
    ADC $22
    STA $22
    DEY
    BNE mul_loop_1
mul_end_2:
    LDA $22
    RTS
    * = $9200
    ; Function: process
    ;   Params: data: *u8, count: u16
    ;   Returns: bool
    ;   Location: $9200
    ;   Attributes: CODE
process:
    ; Inline: add(...) [2 args]
    LDA #$05
    STA $50
    LDA #$03
    STA $51
    LDA $41
    STA $20
    LDA $40
    CLC
    ADC $20
    STA $46
    ; Call: multiply(...) [2 args]
    STA $50
    LDA #$02
    STA $51
    JSR multiply
    STA $47
    STA SCREEN
    LDA #$01
    RTS
    * = $9300
    ; Function: main
    ;   Params: none
    ;   Returns: void
    ;   Location: $9300
main:
    ; Call: process(...) [2 args]
    LDA #$00
    STA $50
    LDA #$0A
    STA $51
    JSR process
    STA $48
    RTS
