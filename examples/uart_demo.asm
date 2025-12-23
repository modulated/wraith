UART_FCR = $9002
UART_RBR = $9000
UART_LSR = $9005
UART_MCR = $9004
LCR_8N1 = $0003
FCR_TX_RST = $0004
LCR_DLAB = $0080
LSR_DATA_RDY = $0001
UART_IER = $9001
BAUD_9600 = $000C
MCR_DTR = $0001
LSR_THR_EMPTY = $0020
UART_LCR = $9003
FCR_RX_RST = $0002
FCR_TRIGGER = $00C0
UART_DLL = $9000
FCR_ENABLE = $0001
UART_THR = $9000
MCR_RTS = $0002
UART_DLM = $9001
UART_BASE = $9000
UART_RBR = $9000
UART_THR = $9000
UART_DLL = $9000
UART_DLM = $9001
UART_IER = $9001
UART_IIR = $9002
UART_FCR = $9002
UART_LCR = $9003
UART_MCR = $9004
UART_LSR = $9005
UART_MSR = $9006
UART_SCR = $9007
LCR_DLAB = $0080
LCR_8N1 = $0003
FCR_ENABLE = $0001
FCR_RX_RST = $0002
FCR_TX_RST = $0004
FCR_TRIGGER = $00C0
LSR_DATA_RDY = $0001
LSR_THR_EMPTY = $0020
MCR_DTR = $0001
MCR_RTS = $0002
BAUD_9600 = $000C
BAUD_19200 = $0006
BAUD_38400 = $0003
BAUD_115200 = $0001
    * = $9000
    ; Function: uart_init
    ;   Params: none
    ;   Returns: void
    ;   Location: $9000
uart_init:
    LDA LCR_DLAB
    STA UART_LCR
    LDA BAUD_9600
    STA UART_DLL
    LDA #$00
    STA UART_DLM
    LDA LCR_8N1
    STA UART_LCR
    LDA FCR_RX_RST
    STA $20
    LDA FCR_ENABLE
    ORA $20
    TAY
    LDA FCR_TX_RST
    STA $20
    TYA
    ORA $20
    TAY
    LDA FCR_TRIGGER
    STA $20
    TYA
    ORA $20
    STA UART_FCR
    LDA MCR_RTS
    STA $20
    LDA MCR_DTR
    ORA $20
    STA UART_MCR
    LDA #$00
    STA UART_IER
    RTS
    * = $9100
    ; Function: uart_wait_tx
    ;   Params: none
    ;   Returns: void
    ;   Location: $9100
uart_wait_tx:
    LDA #$00
    STA $40
while_start_1:
    LDA LSR_THR_EMPTY
    STA $20
    LDA $40
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
    * = $9200
    ; Function: uart_putc
    ;   Params: ch: u8
    ;   Returns: void
    ;   Location: $9200
uart_putc:
    ; Call: uart_wait_tx()
    JSR uart_wait_tx
    LDA $41
    STA UART_THR
    RTS
    * = $9300
    ; Function: uart_data_ready
    ;   Params: none
    ;   Returns: u8
    ;   Location: $9300
uart_data_ready:
    LDA UART_LSR
    STA $42
    LDA LSR_DATA_RDY
    STA $20
    LDA $42
    AND $20
    RTS
    * = $9400
    ; Function: uart_getc
    ;   Params: none
    ;   Returns: u8
    ;   Location: $9400
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
    * = $9500
    ; Function: uart_puts_simple
    ;   Params: msg: u8
    ;   Returns: void
    ;   Location: $9500
uart_puts_simple:
    ; Call: uart_putc(...) [1 arg]
    LDA $43
    STA $50
    JSR uart_putc
    RTS
    * = $9600
    ; Function: uart_newline
    ;   Params: none
    ;   Returns: void
    ;   Location: $9600
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
    * = $9700
    ; Function: uart_print_hex
    ;   Params: value: u8
    ;   Returns: void
    ;   Location: $9700
uart_print_hex:
    LDA #$04
    STA $20
    LDA $44
    LDX $20
    CPX #$00
    BEQ shr_end_10
shr_loop_9:
    LSR A
    DEX
    BNE shr_loop_9
shr_end_10:
    STA $45
    LDA #$0F
    STA $20
    LDA $44
    AND $20
    STA $46
    LDA #$0A
    STA $20
    LDA $45
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
    LDA $46
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
    * = $9800
    ; Function: uart_echo_demo
    ;   Params: none
    ;   Returns: void
    ;   Location: $9800
uart_echo_demo:
    LDA #$00
    STA $47
    ; Call: uart_putc(...) [1 arg]
    LDA #$48
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$65
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$6C
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$6C
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$6F
    STA $50
    JSR uart_putc
    ; Call: uart_newline()
    JSR uart_newline
while_start_19:
    LDA #$01
    CMP #$00
    BEQ while_end_20
    ; Call: uart_getc()
    JSR uart_getc
    STA $47
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
    LDA #$49
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$4E
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$49
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$54
    STA $50
    JSR uart_putc
    ; Call: uart_newline()
    JSR uart_newline
    ; Call: uart_echo_demo()
    JSR uart_echo_demo
    ; Call: uart_putc(...) [1 arg]
    LDA #$44
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$4F
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$4E
    STA $50
    JSR uart_putc
    ; Call: uart_putc(...) [1 arg]
    LDA #$45
    STA $50
    JSR uart_putc
    ; Call: uart_newline()
    JSR uart_newline
    RTS
