LED = $6000
counter:
    ; static counter: 0
    * = $9000
    ; Function: main
    ;   Params: none
    ;   Returns: void
    ;   Location: $9000
main:
    LDA #$64
    STA LED
    LDA #$0C
    STA $0000
    STA LED
    RTS
