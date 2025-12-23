LED = $6000
LED = $6000
MAX_COUNT:
    ; static MAX_COUNT: 100
BAUD_RATE:
    ; static BAUD_RATE: 12
BUFFER_SIZE:
    ; static BUFFER_SIZE: 256
counter:
    ; static counter: 0
    * = $9000
    ; Function: main
    ;   Params: none
    ;   Returns: void
    ;   Location: $9000
main:
    LDA $0000
    STA LED
    LDA $0000
    STA $0000
    STA LED
    RTS
