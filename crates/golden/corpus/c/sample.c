#include <stdio.h>
#include <stdlib.h>

struct point {
    int x;
    int y;
};

static int add(int a, int b) {
    return a + b;
}

int main(void) {
    struct point p = { .x = 1, .y = 2 };
    int sum = add(p.x, p.y);
    printf("sum=%d\n", sum);
    return sum;
}
