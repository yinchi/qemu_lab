#ifndef UART_IO_H
#define UART_IO_H

#include "registers.h"

/* Confirmed via device tree dump: PL011 is wired to SPI 1 -> Interrupt ID 32+1 */
#define UART0_IRQ 33

/** Transmit a character via UART0 */
static void uart_putc(char c)
{
	while (UART0_FR & UART0_FR_TXFF)
		;
	UART0_DR = (unsigned int)c;
}

/** Transmit a string via UART0 */
static void uart_puts(const char *s)
{
	while (*s) {
		uart_putc(*s++);
	}
}

/** Transmit a hexadecimal value via UART0 as a "0x..." string */
static void uart_puthex(unsigned long v)
{
	static const char digits[] = "0123456789abcdef";
	int shift;

	uart_puts("0x");
	for (shift = 60; shift >= 0; shift -= 4)
		uart_putc(digits[(v >> shift) & 0xf]);
}

#endif /* UART_IO_H */