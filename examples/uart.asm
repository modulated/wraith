UART_DLL = $7F00
UART_LSR = $7F05
UART_RBR = $7F00
UART_LCR = $7F03
UART_THR = $7F00
UART_MCR = $7F04
UART_IER = $7F01
UART_DLM = $7F01
UART_FCR = $7F02
    * = $9000
    ; Function: uart_init
    ;   Params: none
    ;   Returns: void
    ;   Location: $9000
uart_init:
    LDA #$80
    STA UART_LCR
    LDA #$0C
    STA UART_DLL
    LDA #$00
    STA UART_DLM
    LDA #$03
    STA UART_LCR
    LDA #$C7
    STA UART_FCR
    LDA #$03
    STA UART_MCR
    LDA #$00
    STA UART_IER
    RTS
    * = $902E
    ; Function: uart_wait_tx
    ;   Params: none
    ;   Returns: void
    ;   Location: $902E
uart_wait_tx:
    LDA #$00
    STA $40
while_start_1:
    LDA #$20
    STA $20
    AND $20
    TAY
    LDA #$00
    STA $20
    TYA
    CMP $20
    BEQ eq_true_3
    LDA #$00
    JMP eq_end_4
eq_true_3:
    LDA #$01
eq_end_4:
    CMP #$00
    BEQ while_end_2
    LDA UART_LSR
    STA $40
    JMP while_start_1
while_end_2:
    RTS
    * = $9060
    ; Function: uart_putc
    ;   Params: ch: u8
    ;   Returns: void
    ;   Location: $9060
uart_putc:
    ; Call: uart_wait_tx()
    JSR uart_wait_tx
    LDA $41
    STA UART_THR
    RTS
    * = $9073
    ; Function: uart_newline
    ;   Params: none
    ;   Returns: void
    ;   Location: $9073
uart_newline:
    ; Call: uart_putc(...) [1 arg]
    LDA #$0D
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$0A
    STA $50
    JSR uart_putc
    RTS
    * = $908C
    ; Function: uart_data_ready
    ;   Params: none
    ;   Returns: u8
    ;   Location: $908C
uart_data_ready:
    LDA UART_LSR
    STA $42
    LDA #$01
    STA $20
    AND $20
    RTS
    * = $90A2
    ; Function: uart_getc
    ;   Params: none
    ;   Returns: u8
    ;   Location: $90A2
uart_getc:
while_start_5:
    ; Call: uart_data_ready()
    JSR uart_data_ready
    TAY
    LDA #$00
    STA $20
    TYA
    CMP $20
    BEQ eq_true_7
    LDA #$00
    JMP eq_end_8
eq_true_7:
    LDA #$01
eq_end_8:
    CMP #$00
    BEQ while_end_6
    JMP while_start_5
while_end_6:
    LDA UART_RBR
    RTS
    * = $90CB
    ; Function: uart_print_hex
    ;   Params: value: u8
    ;   Returns: void
    ;   Location: $90CB
uart_print_hex:
    LDA #$04
    STA $20
    LDA $43
    LDX $20
    CPX #$00
    BEQ shr_end_10
shr_loop_9:
    LSR A
    DEX
    BNE shr_loop_9
shr_end_10:
    STA $44
    LDA #$0F
    STA $20
    LDA $43
    AND $20
    STA $45
    LDA #$0A
    STA $20
    LDA $44
    CMP $20
    BCC lt_true_13
    LDA #$00
    JMP lt_end_14
lt_true_13:
    LDA #$01
lt_end_14:
    CMP #$00
    BEQ else_11
    ; Call: uart_putc(...) [1 arg]
    STA $20
    LDA #$30
    CLC
    ADC $20
    STA $50
    JSR uart_putc
    JMP end_12
else_11:
    ; Call: uart_putc(...) [1 arg]
    STA $20
    LDA #$37
    CLC
    ADC $20
    STA $50
    JSR uart_putc
end_12:
    LDA #$0A
    STA $20
    LDA $45
    CMP $20
    BCC lt_true_17
    LDA #$00
    JMP lt_end_18
lt_true_17:
    LDA #$01
lt_end_18:
    CMP #$00
    BEQ else_15
    ; Call: uart_putc(...) [1 arg]
    STA $20
    LDA #$30
    CLC
    ADC $20
    STA $50
    JSR uart_putc
    JMP end_16
else_15:
    ; Call: uart_putc(...) [1 arg]
    STA $20
    LDA #$37
    CLC
    ADC $20
    STA $50
    JSR uart_putc
end_16:
    RTS
    * = $9154
    ; Function: echo_loop
    ;   Params: none
    ;   Returns: void
    ;   Location: $9154
echo_loop:
    LDA #$00
    STA $46
    ; Call: uart_putc(...) [1 arg]
    LDA #$3E
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$20
    STA $50
    JSR uart_putc
while_start_19:
    LDA #$01
    CMP #$00
    BEQ while_end_20
    ; Call: uart_getc()
    JSR uart_getc
    STA $46
    ; Call: uart_putc(...) [1 arg]
    STA $50
    JSR uart_putc
    LDA #$0D
    STA $20
    CMP $20
    BEQ eq_true_23
    LDA #$00
    JMP eq_end_24
eq_true_23:
    LDA #$01
eq_end_24:
    CMP #$00
    BEQ else_21
    ; Call: uart_newline()
    JSR uart_newline
    ; Call: uart_putc(...) [1 arg]
    LDA #$3E
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$20
    STA $50
    JSR uart_putc
    JMP end_22
else_21:
end_22:
    LDA #$1B
    STA $20
    CMP $20
    BEQ eq_true_27
    LDA #$00
    JMP eq_end_28
eq_true_27:
    LDA #$01
eq_end_28:
    CMP #$00
    BEQ else_25
    JMP while_end_20
    JMP end_26
else_25:
end_26:
    JMP while_start_19
while_end_20:
    RTS
    * = $8000
    ; Function: main
    ;   Params: none
    ;   Returns: void
    ;   Location: $8000
main:
    ; Call: uart_init()
    JSR uart_init
    ; Call: uart_putc(...) [1 arg]
    LDA #$52
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$65
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$61
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$64
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$79
    STA $50
    JSR uart_putc
    ; Call: uart_newline()
    JSR uart_newline
    ; Call: echo_loop()
    JSR echo_loop
    ; Call: uart_putc(...) [1 arg]
    LDA #$42
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$79
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$65
    STA $50
    JSR uart_putc
    ; Call: uart_newline()
    JSR uart_newline
    RTS
