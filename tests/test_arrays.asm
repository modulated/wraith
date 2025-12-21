Successfully compiled 'tests/test_arrays.wr' to 'tests/test_arrays.asm'
   JMP arr_fill_skip_2
arr_fill_1:
    .byte $00
    .byte $00
    .byte $00
    .byte $00
    .byte $00
arr_fill_skip_2:
    ; Load address of filled array (5 elements)
    LDA #<arr_fill_1
    LDX #>arr_fill_1
    STA $40
    JMP arr_skip_4
arr_3:
    .byte $01
    .byte $02
    .byte $03
arr_skip_4:
    ; Load address of array (3 elements)
    LDA #<arr_3
    LDX #>arr_3
    STA $41
    LDA #$AA
    STA $C000
    RTS
