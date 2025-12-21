Successfully compiled 'tests/test_named_types.wr' to 'tests/test_named_types.asm'
_fill_skip_2
arr_fill_1:
    .byte $00
    .byte $00
    .byte $00
    .byte $00
    .byte $00
    .byte $00
arr_fill_skip_2:
    ; Load address of filled array (6 elements)
    LDA #<arr_fill_1
    LDX #>arr_fill_1
    STA $40
    LDA #$FF
    STA $C000
    RTS
