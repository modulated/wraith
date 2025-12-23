X = $0401
SCREEN = $0400
Y = $0402
    * = $9000
    ; Function: main
    ;   Params: none
    ;   Returns: void
    ;   Location: $9000
main:
    LDA #$0A
    STA X
    LDA #$14
    STA Y
    LDA Y
    STA $20
    LDA X
    CLC
    ADC $20
    STA SCREEN
    RTS
