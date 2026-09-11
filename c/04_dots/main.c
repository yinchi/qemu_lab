#include "registers.h"
#include "uart_io.h"
#include "timer.h"

#define RED     "\033[1;31m"
#define GREEN   "\033[1;32m"
#define RESET   "\033[0m"

/* Modes for the irq_handler */
#define MODE_WAITING 0 /* Waiting for UART input */
#define MODE_PRINTING 1 /* Printing dots, discard UART input */

static int mode = MODE_WAITING; /* Current mode of the irq_handler */
static int dots_to_print = 0; /* Number of dots to print */

static void gic_init(void)
{
    /* Splits UART0_IRQ into the appropriate GICD_ISENABLER register and bit, then
    enable that bit. */
    GICD_ISENABLER(UART0_IRQ / 32) = 1u << (UART0_IRQ % 32);
    GICD_ISENABLER(TIMER_IRQ / 32) = 1u << (TIMER_IRQ % 32);

    /* Set the priority for each interrupt source.  Note that timer interrupts need to
    outrank UART0 interrupts, since the handling of UART0 interrupts involves using the timer for
    delays. */
	GICD_IPRIORITYR(UART0_IRQ) = 0xa0;
    GICD_IPRIORITYR(TIMER_IRQ) = 0x90;

    /* Route each IRQ interrupt to CPU0 */
	GICD_ITARGETSR(UART0_IRQ) = 0x1;
    GICD_ITARGETSR(TIMER_IRQ) = 0x1;

    /* Enable the GIC distributor */
	GICD_CTLR = 1;
    /* Allow through interrupts of any priority (0x00 to 0xff) */
	GICC_PMR = 0xff;
    /* Enable the GIC CPU interface */
	GICC_CTLR = 1;
}

void main(void)
{
    gic_init();
	UART0_IMSC = UART0_RX;           /* enable receive interrupts */
    
    /* DAIF clear, 0x2 bit (IRQ mask) 
    DAIF = Debug, Asynchronous abort, IRQ, FIQ masks
    Clearing the IRQ mask bit allows IRQ exceptions to be taken. */
	asm volatile("msr daifclr, #2");

    uart_puts(GREEN "Select number of dots to print (1-9).\r\n" RESET);
        for (;;)
            asm volatile("wfe");         /* sleep between interrupts */
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
	if (irq == UART0_IRQ && mode == MODE_WAITING) {

        /* Read characters from the receive buffer a 1-9 is encountered. */
		while (!(UART0_FR & UART0_FR_RXFE)) {
            char c = (char)(UART0_DR & 0xff);
            if (c >= '1' && c <= '9') {

                /* Echo back the number of dots selected */
                char num_dots = c - '0';
                uart_putc(c);
                uart_puts("\r\n");

                /* Set up the program state for printing dots */
                dots_to_print = num_dots;
                mode = MODE_PRINTING;

                timer_arm(timer_freq()); /* Arm the timer for 1 second */
                break;
            }
		}

        /* Flush the rest of the receive buffer */
        while (!(UART0_FR & UART0_FR_RXFE)) {
            (void)(UART0_DR & 0xff);
        }

        /* Clear the interrupt */
		UART0_ICR = UART0_RX;
	} else if (irq == UART0_IRQ && mode != MODE_WAITING) {
        /* Input received while not in waiting mode, just empty the receive buffer and
        clear the interrupt */
        while (!(UART0_FR & UART0_FR_RXFE)) {
            (void)(UART0_DR & 0xff);
        }
        UART0_ICR = UART0_RX;
    } else if (irq == TIMER_IRQ && mode == MODE_PRINTING) {
        uart_putc('.');
        dots_to_print--;

        /* Re-arm the timer for the next dot if there are more dots to print */
        if (dots_to_print > 0) {
            timer_arm(timer_freq()); /* Arm the timer for 1 second */
        }

        /* Handle end-of-printing */
        else {
            mode = MODE_WAITING;
            uart_puts("\r\n"); /* End the line of dots */
            uart_puts(GREEN "Select number of dots to print (1-9).\r\n" RESET);
        }
    } else if (irq == TIMER_IRQ && mode != MODE_PRINTING) {
        /* Input received from timer while not in printing mode, just clear the interrupt */
        timer_disable();
    }

    /* Signal end of interrupt to the GIC by writing back `iar` to the end-of-interrupt register */
	GICC_EOIR = iar;
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