/* PL011 UART0 base address on the QEMU "virt" machine */
#define UART0_BASE 0x09000000UL

/** Data register; writing transmits a character, reading receives one */
#define UART0_DR   (*(volatile unsigned int *)(UART0_BASE + 0x00))

/** Flag register; a read-only status register */
#define UART0_FR   (*(volatile unsigned int *)(UART0_BASE + 0x18))

/** Status mask: transmit buffer full? */
#define UART0_FR_TXFF (1 << 5)

/** Status mask: receive buffer empty? */
#define UART0_FR_RXFE (1 << 4)

#define GREEN   "\033[1;32m"
#define RESET   "\033[0m"

/** Output a character to the UART0 console */
static void uart_putc(char c)
{
	while (UART0_FR & UART0_FR_TXFF)
		;
	UART0_DR = (unsigned int)c;
}

/** Output a character string to the UART0 console */
static void uart_puts(const char *s)
{
	while (*s) {
		uart_putc(*s++);
	}
}

/** Block until a character is received from the UART0 console */
static char uart_getc(void)
{
	while (1) {
		while (UART0_FR & UART0_FR_RXFE)
			;
		char c = (char)(UART0_DR & 0xFF);
    
        /** Printable ASCII character or specific special characters only */
        if ((c >= 32 && c <= 126) || c == '\r' || c == '\n' || c == 0x1b || c == '\b')
            return c;
	}
}

void main(void)
{
	uart_puts(GREEN "Echo server. Type characters; they will be echoed back.\r\n" RESET);

	for (;;) {
		char c = uart_getc();
		if (c == '\r' || c == '\n') {uart_puts("\r\n");}
		else if (c == '\b') {uart_puts("\b \b");}
		else if (c == 0x1b) {uart_putc('^');}
		else {uart_putc(c);}
	}
}
