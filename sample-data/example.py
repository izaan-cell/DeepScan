def calculate_fibonacci(n):
    """Returns the nth Fibonacci number using recursion."""
    if n <= 1:
        return n
    return calculate_fibonacci(n - 1) + calculate_fibonacci(n - 2)
