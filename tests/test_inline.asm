    * = $9000
add:
    LDA $41
    STA $20
    LDA $40
    CLC
    ADC $20
    RTS
    * = $9100
double:
    LDA $42
    STA $20
    CLC
    ADC $20
    RTS
    * = $9200
regular_add:
    LDA $44
    STA $20
    LDA $43
    CLC
    ADC $20
    RTS
    * = $9300
main:
    ; Inline add
    LDA #$05
    STA $50
    LDA #$03
    STA $51
    LDA $41
    STA $20
    LDA $40
    CLC
    ADC $20
    STA $45
    ; Inline double
    STA $50
    LDA $42
    STA $20
    CLC
    ADC $20
    STA $46
    ; Call regular_add with 2 args
    STA $50
    LDA #$0A
    STA $51
    JSR regular_add
    STA $47
    RTS
