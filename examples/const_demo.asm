DISPLAY = $6001
LED = $6000
    * = $9000
    ; Function: main
    ;   Params: none
    ;   Returns: void
    ;   Location: $9000
main:
    LDA #$FF
    STA LED
    LDA #$80
    STA DISPLAY
    LDA #$03
    STA $40
    STA LED
    RTS
