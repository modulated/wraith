    * = $0801
main:
    ; Load local x (offset 0)
    PHA
    ; Load local y (offset 0)
    PHA
    JSR sum
    PLA
    PLA
    STA $0000
    RTS
sum:
    ; Load local x (offset 0)
    PHA
    ; Load local y (offset 0)
    STA $20
    PLA
    CLC
    ADC $20
    RTS
    RTS
