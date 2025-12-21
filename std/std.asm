memcpy:
    LDY #$00
    loop_memcpy:
    LDA $41,Y
    STA $40,Y
    INY
    CPY $42
    BNE loop_memcpy
    RTS
memset:
    LDA $44
    LDY #$00
    loop_memset:
    STA $40,Y
    INY
    CPY $42
    BNE loop_memset
    RTS
memcmp:
    LDY #$00
    loop_memcmp:
    LDA $46,Y
    CMP $47,Y
    BNE not_equal
    INY
    CPY $42
    BNE loop_memcmp
    LDA #$01
    RTS
    not_equal:
    LDA #$00
