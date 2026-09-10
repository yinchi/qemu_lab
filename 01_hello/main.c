/* PL011 UART0 base address on the QEMU "virt" machine */
#define UART0_BASE 0x09000000UL

/** Data register; writing to this register transmits a character */
#define UART0_DR   (*(volatile unsigned int *)(UART0_BASE + 0x00))

/** Flag register; a read-only status register */
#define UART0_FR   (*(volatile unsigned int *)(UART0_BASE + 0x18))

/** Status mask: transmit buffer full? */
#define UART0_FR_TXFF (1 << 5)

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
		if (*s == '\n')
			uart_putc('\r');
		uart_putc(*s++);
	}
}

void main(void)
{
	uart_puts(GREEN "Hello, world!\n" RESET);
}
