#!/usr/bin/env python3
"""
HDFC Credit Card Spending Analyzer
Generates monthly breakdown by category and trend analysis with visualization.

Usage:
    python analyze_spending.py [--csv PATH] [--categories PATH] [--output PATH] [--cycle-start DAY]

Options:
    --cycle-start DAY  Day of month when billing cycle starts (1-31).
                       E.g., --cycle-start 16 groups transactions from 16th to 16th of next month.
                       For days 29-31, adjusts for shorter months (Feb uses 28/29, Apr uses 30).
                       Default: 1 (calendar months)

Dependencies:
    pip install matplotlib
"""

import argparse
import csv
import json
import statistics
from collections import defaultdict
from datetime import datetime


def load_transactions(csv_path):
    """Load transactions from CSV file."""
    transactions = []
    with open(csv_path, "r") as f:
        reader = csv.DictReader(f)
        for row in reader:
            transactions.append(row)
    return transactions


def load_categories(categories_path):
    """Load category patterns from JSON file."""
    with open(categories_path, "r") as f:
        return json.load(f)


def categorize(desc, categories):
    """Categorize a transaction based on description patterns."""
    desc_upper = desc.upper()
    for category, patterns in categories.items():
        for pattern in patterns:
            if pattern.upper() in desc_upper:
                return category
    return "Uncategorized"


def get_days_in_month(year, month):
    """Return the number of days in a given month."""
    if month == 12:
        next_month = datetime(year + 1, 1, 1)
    else:
        next_month = datetime(year, month + 1, 1)
    return (next_month - datetime(year, month, 1)).days


def get_effective_cycle_day(year, month, cycle_start_day):
    """Get the effective cycle start day for a given month.

    For months with fewer days than cycle_start_day, returns the last day of the month.
    E.g., cycle_start_day=31 in February returns 28 (or 29 in leap year).
    """
    days_in_month = get_days_in_month(year, month)
    return min(cycle_start_day, days_in_month)


def get_cycle_key(date, cycle_start_day):
    """Get the billing cycle key for a given date.

    If cycle_start_day is 1, uses calendar months.
    Otherwise, a cycle runs from cycle_start_day of one month to cycle_start_day of next month.
    The cycle is named after the month in which it starts.

    For cycle_start_day > 28, adjusts for shorter months (Feb, 30-day months).
    E.g., with cycle_start_day=31:
    - Jan 31 to Feb 28/29 -> "2024-01" (Jan cycle)
    - Mar 1 to Mar 30 -> "2024-02" (Feb cycle, since Feb ends early)
    - Mar 31 to Apr 30 -> "2024-03" (Mar cycle)

    Example with cycle_start_day=16:
    - Jan 16 to Feb 16 -> "2024-01" (Jan cycle)
    - Feb 16 to Mar 16 -> "2024-02" (Feb cycle)
    """
    if cycle_start_day == 1:
        return f"{date.year}-{date.month:02d}"

    # Get effective cycle day for this month (handles shorter months)
    effective_day = get_effective_cycle_day(date.year, date.month, cycle_start_day)

    # If date is on or before the effective cycle day, it belongs to previous month's cycle
    # If date is after the effective cycle day, it belongs to current month's cycle
    if date.day <= effective_day:
        # Move to previous month
        if date.month == 1:
            return f"{date.year - 1}-12"
        else:
            return f"{date.year}-{date.month - 1:02d}"
    else:
        return f"{date.year}-{date.month:02d}"


def process_transactions(transactions, categories, cycle_start_day=1):
    """Process transactions and compute monthly data."""
    monthly_data = defaultdict(lambda: defaultdict(float))
    monthly_totals = defaultdict(float)

    for t in transactions:
        amount = float(t["Amount"])
        desc = t["Description"]
        date_str = t["Date"]

        # Skip credits/payments
        if amount >= 0:
            continue
        if (
            "CREDIT CARD PAYMENT" in desc.upper()
            or "CC PAYMENT" in desc.upper()
            or "NETBANKING TRANSFER" in desc.upper()
        ):
            continue

        # Extract date and get cycle key
        date = datetime.strptime(date_str.split()[0], "%Y-%m-%d")
        month_key = get_cycle_key(date, cycle_start_day)

        category = categorize(desc, categories)
        spend = abs(amount)

        monthly_data[month_key][category] += spend
        monthly_totals[month_key] += spend

    return monthly_data, monthly_totals


def print_text_report(monthly_data, monthly_totals):
    """Print text-based analysis report."""
    sorted_months = sorted(monthly_data.keys())

    print("=" * 100)
    print("                         MONTHLY SPENDING BY CATEGORY (₹)")
    print("=" * 100)

    header_cats = [
        "Travel",
        "Shopping",
        "Amazon",
        "Food",
        "Health",
        "Fuel",
        "Util",
        "Grocer",
        "Uncat",
        "TOTAL",
    ]
    print(f"{'Month':<10}", end="")
    for cat in header_cats:
        print(f"{cat:>10}", end="")
    print()
    print("-" * 110)

    for month in sorted_months:
        data = monthly_data[month]
        total = monthly_totals[month]
        print(f"{month:<10}", end="")

        display_cats = [
            "Travel",
            "Shopping",
            "Amazon",
            "Food & Dining",
            "Healthcare",
            "Fuel",
            "Utilities",
            "Groceries",
            "Uncategorized",
        ]
        for cat in display_cats:
            val = data.get(cat, 0)
            print(f"{val:>10.0f}", end="")
        print(f"{total:>10.0f}")

    print("-" * 110)

    # Monthly trend bar chart
    print("\n" + "=" * 70)
    print("                MONTHLY SPENDING TREND")
    print("=" * 70)

    totals_list = [(m, monthly_totals[m]) for m in sorted_months]
    max_spend = max(t[1] for t in totals_list)
    bar_width = 40

    for month, total in totals_list:
        bar_len = int((total / max_spend) * bar_width)
        bar = "█" * bar_len
        print(f"{month} ₹{total:>10,.0f} │{bar}")

    # Month-over-month analysis
    print("\n" + "=" * 70)
    print("                MONTH-OVER-MONTH CHANGE")
    print("=" * 70)

    print(f"\n{'Month':<12} {'Spending':>12} {'Change':>14} {'% Change':>10}")
    print("-" * 50)

    for i, (month, total) in enumerate(totals_list):
        if i == 0:
            print(f"{month:<12} ₹{total:>10,.0f}            -          -")
        else:
            prev = totals_list[i - 1][1]
            change = total - prev
            pct_change = (change / prev) * 100 if prev != 0 else 0
            arrow = "↑" if change > 0 else "↓" if change < 0 else "→"
            print(
                f"{month:<12} ₹{total:>10,.0f}  {arrow} ₹{abs(change):>9,.0f}  {pct_change:>+8.1f}%"
            )

    # Trend analysis
    print("\n" + "=" * 70)
    print("                TREND ANALYSIS")
    print("=" * 70)

    totals_only = [t[1] for t in totals_list]
    mid = len(totals_only) // 2

    first_half_avg = sum(totals_only[:mid]) / mid if mid > 0 else 0
    second_half_avg = (
        sum(totals_only[mid:]) / (len(totals_only) - mid)
        if len(totals_only) > mid
        else 0
    )

    first_months = sorted_months[:mid]
    second_months = sorted_months[mid:]

    print(
        f"\nFirst half ({first_months[0]} to {first_months[-1]}):  ₹{first_half_avg:>10,.0f}/month avg"
    )
    print(
        f"Second half ({second_months[0]} to {second_months[-1]}): ₹{second_half_avg:>10,.0f}/month avg"
    )

    if second_half_avg < first_half_avg:
        pct = ((first_half_avg - second_half_avg) / first_half_avg) * 100
        print(f"\n✅ GOOD NEWS: Spending DECREASED by {pct:.1f}% in second half")
    else:
        pct = ((second_half_avg - first_half_avg) / first_half_avg) * 100
        print(f"\n⚠️  Spending INCREASED by {pct:.1f}% in second half")

    # Category trends
    print("\n" + "=" * 70)
    print("          CATEGORY TREND (First vs Second Half)")
    print("=" * 70)

    print(f"\n{'Category':<20} {'1st Half Avg':>14} {'2nd Half Avg':>14} {'Trend':>12}")
    print("-" * 65)

    for cat in [
        "Travel",
        "Shopping",
        "Amazon",
        "Food & Dining",
        "Healthcare",
        "Fuel",
        "Utilities",
        "Groceries",
        "Uncategorized",
    ]:
        fh_sum = sum(monthly_data[m].get(cat, 0) for m in first_months)
        sh_sum = sum(monthly_data[m].get(cat, 0) for m in second_months)

        fh_avg = fh_sum / len(first_months) if first_months else 0
        sh_avg = sh_sum / len(second_months) if second_months else 0

        if fh_avg > 100:
            pct = ((sh_avg - fh_avg) / fh_avg) * 100
            if pct > 15:
                trend = f"↑ +{pct:.0f}%"
            elif pct < -15:
                trend = f"↓ {pct:.0f}%"
            else:
                trend = f"→ {pct:+.0f}%"
        else:
            trend = "-"

        print(f"{cat:<20} ₹{fh_avg:>12,.0f} ₹{sh_avg:>12,.0f}   {trend}")

    # Key insights
    print("\n" + "=" * 70)
    print("                KEY INSIGHTS")
    print("=" * 70)

    sorted_by_spend = sorted(totals_list, key=lambda x: x[1], reverse=True)
    print(f"\n🔴 Highest spending months:")
    for m, t in sorted_by_spend[:3]:
        print(f"   {m}: ₹{t:,.0f}")

    print(f"\n🟢 Lowest spending months:")
    for m, t in sorted_by_spend[-3:]:
        print(f"   {m}: ₹{t:,.0f}")

    # Volatility
    std_dev = statistics.stdev(totals_only)
    mean_spend = statistics.mean(totals_only)
    cv = (std_dev / mean_spend) * 100

    print(f"\n📊 Spending consistency:")
    print(f"   Average monthly: ₹{mean_spend:,.0f}")
    print(f"   Std deviation:   ₹{std_dev:,.0f}")
    print(f"   Volatility (CV): {cv:.1f}%")

    if cv > 50:
        print("   ⚠️  High volatility - spending varies significantly month to month")
    elif cv > 30:
        print("   ⚡ Moderate volatility - some variation in monthly spending")
    else:
        print("   ✅ Low volatility - consistent spending pattern")


def generate_graph(monthly_data, monthly_totals, output_path):
    """Generate matplotlib visualization."""
    try:
        import matplotlib.pyplot as plt
        import matplotlib.ticker as ticker
    except ImportError:
        print("\n⚠️  matplotlib not installed. Run: pip install matplotlib")
        return

    sorted_months = sorted(monthly_data.keys())
    month_labels = [m.split("-")[1] for m in sorted_months]  # Just month number

    fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(12, 10))
    fig.suptitle("Credit Card Spending Analysis", fontsize=14, fontweight="bold")

    # Plot 1: Monthly Total Trend
    totals = [monthly_totals[m] for m in sorted_months]
    colors_trend = [
        "#ff6b6b" if t > 200000 else "#4ecdc4" if t < 150000 else "#ffe66d"
        for t in totals
    ]

    bars1 = ax1.bar(
        month_labels, totals, color=colors_trend, edgecolor="black", linewidth=0.5
    )
    ax1.plot(month_labels, totals, "ko-", linewidth=2, markersize=8)

    for bar, total in zip(bars1, totals):
        ax1.text(
            bar.get_x() + bar.get_width() / 2,
            bar.get_height() + 5000,
            f"₹{total/1000:.0f}K",
            ha="center",
            va="bottom",
            fontsize=9,
        )

    ax1.set_ylabel("Spending (₹)", fontsize=11)
    ax1.set_title("Monthly Total Spending", fontsize=12)
    ax1.yaxis.set_major_formatter(ticker.FuncFormatter(lambda x, p: f"₹{x/1000:.0f}K"))
    ax1.axhline(
        y=sum(totals) / len(totals),
        color="red",
        linestyle="--",
        label=f"Avg: ₹{sum(totals)/len(totals)/1000:.0f}K",
    )
    ax1.legend()
    ax1.set_ylim(0, max(totals) * 1.15)

    # Plot 2: Stacked bar by category
    key_categories = [
        "Travel",
        "Shopping",
        "Amazon",
        "Food & Dining",
        "Healthcare",
        "Fuel",
        "Utilities",
        "Groceries",
        "Uncategorized",
    ]
    category_colors = [
        "#3498db",
        "#e74c3c",
        "#f39c12",
        "#2ecc71",
        "#9b59b6",
        "#1abc9c",
        "#34495e",
        "#e67e22",
        "#95a5a6",
    ]

    bottom = [0] * len(sorted_months)
    for cat, color in zip(key_categories, category_colors):
        values = [monthly_data[m].get(cat, 0) for m in sorted_months]
        ax2.bar(
            month_labels,
            values,
            bottom=bottom,
            label=cat,
            color=color,
            edgecolor="white",
            linewidth=0.5,
        )
        bottom = [b + v for b, v in zip(bottom, values)]

    ax2.set_xlabel("Month", fontsize=11)
    ax2.set_ylabel("Spending (₹)", fontsize=11)
    ax2.set_title("Monthly Spending by Category", fontsize=12)
    ax2.yaxis.set_major_formatter(ticker.FuncFormatter(lambda x, p: f"₹{x/1000:.0f}K"))
    ax2.legend(loc="upper right", fontsize=8, ncol=3)

    plt.tight_layout()
    plt.savefig(output_path, dpi=150, bbox_inches="tight")
    print(f"\n📊 Graph saved to: {output_path}")


def main():
    parser = argparse.ArgumentParser(description="Analyze HDFC credit card spending")
    parser.add_argument(
        "--csv", default="dump.csv", help="Path to transaction CSV file"
    )
    parser.add_argument(
        "--categories", default="custom.json", help="Path to categories JSON file"
    )
    parser.add_argument(
        "--output", default="spending_analysis.png", help="Output path for graph"
    )
    parser.add_argument("--no-graph", action="store_true", help="Skip graph generation")
    parser.add_argument(
        "--cycle-start",
        type=int,
        default=1,
        metavar="DAY",
        help="Day of month when billing cycle starts (1-31, default: 1 for calendar months)",
    )

    args = parser.parse_args()

    if not 1 <= args.cycle_start <= 31:
        parser.error("--cycle-start must be between 1 and 31")

    # Load data
    transactions = load_transactions(args.csv)
    categories = load_categories(args.categories)

    # Process
    monthly_data, monthly_totals = process_transactions(
        transactions, categories, args.cycle_start
    )

    # Print report
    print_text_report(monthly_data, monthly_totals)

    # Generate graph
    if not args.no_graph:
        generate_graph(monthly_data, monthly_totals, args.output)


if __name__ == "__main__":
    main()
