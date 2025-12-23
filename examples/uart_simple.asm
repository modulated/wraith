BAUD_DIV = $000C
UART_THR = $9000
UART_DLL = $9000
LSR_THRE = $0020
UART_LCR = $9003
LCR_8N1 = $0003
UART_DLM = $9001
UART_LSR = $9005
LCR_DLAB = $0080
UART_BASE = $9000
UART_THR = $9000
UART_LSR = $9005
UART_LCR = $9003
UART_DLL = $9000
UART_DLM = $9001
LCR_DLAB = $0080
LCR_8N1 = $0003
LSR_THRE = $0020
BAUD_DIV = $000C
    * = $9000
    ; Function: init_uart
    ;   Params: none
    ;   Returns: void
    ;   Location: $9000
init_uart:
    LDA LCR_DLAB
    STA UART_LCR
    LDA BAUD_DIV
    STA UART_DLL
    LDA #$00
    STA UART_DLM
    LDA LCR_8N1
    STA UART_LCR
    RTS
    * = $9100
    ; Function: wait_ready
    ;   Params: none
    ;   Returns: void
    ;   Location: $9100
wait_ready:
    LDA #$00
    STA $40
while_start_1:
    LDA LSR_THRE
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
    ; Function: send_byte
    ;   Params: data: u8
    ;   Returns: void
    ;   Location: $9200
send_byte:
    ; Call: wait_ready()
    JSR wait_ready
    LDA $41
    STA UART_THR
    RTS
    * = $9300
    ; Function: send_hello
    ;   Params: none
    ;   Returns: void
    ;   Location: $9300
send_hello:
    ; Call: send_byte(...) [1 arg]
    LDA #$48
    STA $50
    JSR send_byte
    ; Call: send_byte(...) [1 arg]
    LDA #$65
    STA $50
    JSR send_byte
    ; Call: send_byte(...) [1 arg]
    LDA #$6C
    STA $50
    JSR send_byte
    ; Call: send_byte(...) [1 arg]
    LDA #$6C
    STA $50
    JSR send_byte
    ; Call: send_byte(...) [1 arg]
    LDA #$6F
    STA $50
    JSR send_byte
    ; Call: send_byte(...) [1 arg]
    LDA #$0D
    STA $50
    JSR send_byte
    ; Call: send_byte(...) [1 arg]
    LDA #$0A
    STA $50
    JSR send_byte
    RTS
    * = $8000
    ; Function: main
    ;   Params: none
    ;   Returns: void
    ;   Location: $8000
main:
    ; Call: init_uart()
    JSR init_uart
    ; Call: send_hello()
    JSR send_hello
    ; Call: send_byte(...) [1 arg]
    LDA #$2A
    STA $50
    JSR send_byte
    RTS
