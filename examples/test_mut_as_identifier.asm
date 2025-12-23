LED = $6000
    * = $9000
    ; Function: main
    ;   Params: none
    ;   Returns: void
    ;   Location: $9000
main:
    LDA #$2A
    STA $40
    STA LED
    RTS
