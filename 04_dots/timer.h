#ifndef TIMER_H
#define TIMER_H

/* Three system registers for the ARM generic timer:
- CNTFRQ_EL0 (counter frequency)
- CNTP_CTL_EL0 (physical timer control)
- CNTP_TVAL_EL0 (timer value).

Not memory mapped, accessed via `msr` and `mrs` instructions. */

#define CNTP_CTL_ENABLE  (1u << 0) /* Timer enabled */
#define CNTP_CTL_IMASK   (1u << 1) /* Mask this timer's own interrupt output, independent of DAIF.I */
#define CNTP_CTL_ISTATUS (1u << 2) /* Read-only: comparator condition currently met */

/* non-secure-phys timer: PPI 14 -> Interrupt ID 16+14 = 30 */
#define TIMER_IRQ 30

/** Read CNTFRQ_EL0: the counter frequency in Hz, fixed by the platform at boot. */
static inline unsigned long timer_freq(void)
{
    unsigned long v;
    asm volatile("mrs %0, cntfrq_el0" : "=r"(v));
    return v;
}

/** Arm (or re-arm) the physical timer to fire `ticks` counter cycles from now. */
static inline void timer_arm(unsigned long ticks)
{
    asm volatile("msr cntp_tval_el0, %0" :: "r"(ticks));
    asm volatile("msr cntp_ctl_el0, %0" :: "r"((unsigned long)CNTP_CTL_ENABLE));
}

/** Disable the physical timer, deasserting its interrupt line. */
static inline void timer_disable(void)
{
    asm volatile("msr cntp_ctl_el0, %0" :: "r"(0UL));
}

#endif /* TIMER_H */