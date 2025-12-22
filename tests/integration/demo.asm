SCREEN = $0400
    ; Memory-mapped SCREEN (read-write)
LED = $0401
    ; Memory-mapped LED (read-write)
    * = $9000
main:
    LDA #$0A
    STA $40
    LDA #$14
    STA $41
    ; Call add with 2 args
    LDA $40
    STA $50
    LDA $41
    STA $51
    JSR add
    STA $42
    STA $0400
    LDA #$0F
    STA $20
    LDA $42
    CMP $20
    BEQ gt_end_4
    BCS gt_true_3
    LDA #$00
    JMP gt_end_4
gt_true_3:
    LDA #$01
gt_end_4:
    CMP #$00
    BEQ else_1
    LDA #$01
    STA $0400
    LDA #$02
    STA $0401
    JMP end_2
else_1:
    LDA #$04
    STA $0401
end_2:
    RTS
    * = $9100
add:
    LDA $44
    STA $20
    LDA $43
    CLC
    ADC $20
    RTS
