    * = $8000
test:
    LDA $0000
    STA $40
    LDA $0000
    STA $41
    LDA $40
    STA $C001
    LDA $41
    STA $C001
    RTS
