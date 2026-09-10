/* PL011 UART0 base address on the QEMU "virt" machine */
#define UART0_BASE 0x09000000UL

/** Data register; writing transmits a character, reading receives one */
#define UART0_DR   (*(volatile unsigned int *)(UART0_BASE + 0x00))

/** Flag register; a read-only status register */
#define UART0_FR   (*(volatile unsigned int *)(UART0_BASE + 0x18))

/* Interrupt Mask Set/Clear register; writing 1 to a bit here enables the interrupt. */
#define UART0_IMSC (*(volatile unsigned int *)(UART0_BASE + 0x38))

/* Interrupt Clear register; writing 1 to a bit here clears the interrupt. */
#define UART0_ICR  (*(volatile unsigned int *)(UART0_BASE + 0x44))

#define UART0_FR_TXFF (1 << 5) /* Status mask: transmit buffer full? */
#define UART0_FR_RXFE (1 << 4) /* Status mask: receive buffer empty? */
#define UART0_RX    (1 << 4)   /* Interrupt mask: receive */

/* Confirmed via device tree dump: PL011 is wired to SPI 1 -> Interrupt ID 32+1 */
#define UART0_IRQ 33

/* Generic Interrupt Controller v2 distributor, confirmed at 0x08000000 via device tree dump */
#define GICD_BASE 0x08000000UL
/* GICv2 distributor control register - master control for the entire distributor */
#define GICD_CTLR          (*(volatile unsigned int *)(GICD_BASE + 0x000))
/* GICv2 distributor (i)nterrupt (s)et-(enable) (r)egisters - each `int` sets 32 interrupts;
e.g. to enable interrupt 33, use `GICD_ISENABLER(1) = (1 << 1);`. Writing 0 bits
does not disable any interrupts. */
#define GICD_ISENABLER(n)  (*(volatile unsigned int *)(GICD_BASE + 0x100 + 4 * (n)))
/* GICv2 distributor interrupt priority registers - 1 byte (`char`) per interrupt from base,
priority value are thus 0-255 */
#define GICD_IPRIORITYR(n) (*(volatile unsigned char *)(GICD_BASE + 0x400 + (n)))
/* GICv2 distributor interrupt processor targets registers - 1 byte (`char`) per interrupt
from base; each `char` specifies the target CPU(s), 0-7 as bit flags */
#define GICD_ITARGETSR(n)  (*(volatile unsigned char *)(GICD_BASE + 0x800 + (n)))

/* GICv2 CPU interface, confirmed at 0x08010000 via device tree dump */
#define GICC_BASE 0x08010000UL
/* GICv2 CPU interface control register */
#define GICC_CTLR (*(volatile unsigned int *)(GICC_BASE + 0x000))
/* Interrupt Priority Mask Register - only interrupts with higher priority than this value are
signaled */
#define GICC_PMR  (*(volatile unsigned int *)(GICC_BASE + 0x004))
/* Interrupt Acknowledge Register - holds the interrupt ID of the highest priority pending
interrupt */
#define GICC_IAR  (*(volatile unsigned int *)(GICC_BASE + 0x00C))
/* End of Interrupt Register - writing the interrupt ID to it clears the interrupt */
#define GICC_EOIR (*(volatile unsigned int *)(GICC_BASE + 0x010))

#define GREEN   "\033[1;32m"
#define RESET   "\033[0m"


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

/** Echo a received character via UART0, with handling for special characters */
static void handle_char(char c)
{
    /** Printable ASCII character */
    if (c >= 32 && c <= 126)
        uart_putc(c);
    
    if (c == '\r' || c == '\n') {
        /** Convert carriage return or newline to \r\n */
        uart_puts("\r\n");
    }

    if (c == 0x1b) {
        uart_putc('^'); /** Show escape character */
    }

    if (c == '\b') {
        /** Handle backspace within a single line. Erases the previous character with a space,
        then moves the cursor back to the newly erased position. No effect if already at the
        beginning of the line: can't backspace further, simply writes a space to column 0
		which is already blank. */
        uart_puts("\b \b");
    }
}

/* Called from vectors.S for any exception we don't expect to handle */
void unexpected_exception(unsigned long vector)
{
	unsigned long esr, elr;

    /* ESR_EL1: exception syndrome register, reason code for the exception 
       ELR_EL1: exception link register, address of the instruction that caused the exception
    */

    /* Load the system register esr_el1 to some register %0, then bind it to the C variable esr */
	asm volatile("mrs %0, esr_el1" : "=r"(esr));
    /** Same for elr_el1 -> elr */
	asm volatile("mrs %0, elr_el1" : "=r"(elr));

	uart_puts("\r\nUnexpected exception! vector=");
	uart_puthex(vector);
	uart_puts(" ESR_EL1=");
	uart_puthex(esr);
	uart_puts(" ELR_EL1=");
	uart_puthex(elr);
	uart_puts("\r\n");

    /* Hang the CPU */
	for (;;)
		asm volatile("wfe");
}

/* Called from vectors.S when a GIC-routed IRQ is taken at EL1 */
void irq_handler(void)
{
    /* Fetch the interrupt acknowledge register */
	unsigned int iar = GICC_IAR;
    /* Extract the interrupt ID (lowest 10 bits) */
	unsigned int irq = iar & 0x3ff;

    /* If the interrupt is from UART0, handle it. Since we only set the "receive" bit in the
    interrupt mask (UART0_IMSC), any UART0 interrupt must be a receive interrupt: there is something
	in the receive buffer. */
	if (irq == UART0_IRQ) {

        /* Read characters from the receive buffer until it is empty and echo them. */
		while (!(UART0_FR & UART0_FR_RXFE)) {
            char c = (char)(UART0_DR & 0xff);
            handle_char(c);
		}

        /* Clear the interrupt */
		UART0_ICR = UART0_RX;
	}

    /* Signal end of interrupt to the GIC by writing back `iar` to the end-of-interrupt register */
	GICC_EOIR = iar;
}

static void gic_init(void)
{
    /* Splits UART0_IRQ into the appropriate GICD_ISENABLER register and bit, then
    enable that bit. */
	GICD_ISENABLER(UART0_IRQ / 32) = 1u << (UART0_IRQ % 32);

    /* Set the priority for the UART0_IRQ interrupt source - arbitrary since it's the only source
    we have. */
	GICD_IPRIORITYR(UART0_IRQ) = 0xa0;

    /* Route the UART0_IRQ interrupt to CPU0 */
	GICD_ITARGETSR(UART0_IRQ) = 0x1;

    /* Enable the GIC distributor */
	GICD_CTLR = 1;
    /* Allow through interrupts of any priority (0x00 to 0xff) */
	GICC_PMR = 0xff;
    /* Enable the GIC CPU interface */
	GICC_CTLR = 1;
}

void main(void)
{
	uart_puts(GREEN "Echo server. Type characters; they will be echoed back.\r\n" RESET);

	gic_init();
	UART0_IMSC = UART0_RX;           /* enable receive interrupts */

    /* DAIF clear, 0x2 bit (IRQ mask) 
       DAIF = Debug, Asynchronous abort, IRQ, FIQ masks
       Clearing the IRQ mask bit allows IRQ exceptions to be taken. */
	asm volatile("msr daifclr, #2");

	for (;;)
		asm volatile("wfe");         /* sleep between interrupts */
}
