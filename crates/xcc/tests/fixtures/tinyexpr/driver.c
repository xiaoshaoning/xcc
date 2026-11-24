// What tinyexpr is asked to do: parse and evaluate a list of expressions, then compile one with a
// variable bound to it and evaluate that. The expressions in the middle are the two the library
// itself refuses (they need builtins its table does not have), which is why their line prints the
// NaN that `te_interp` returns - and that number is where `NAN`'s sign bit shows.
#include <stdio.h>

#include "tinyexpr.h"

int main(void) {
    static const char *const expressions[] = {
        "1+2*3",  "sin(pi/2)", "sqrt(2)^2", "2^10", "(1+2)*(3+4)", "5%3", "-3+10/4",
        "min(3,7)+max(2,9)", "log(100)/log(10)", "if(1<2, 42, 0)", "3==3", "3!=3",
    };
    int count = (int)(sizeof expressions / sizeof expressions[0]);
    int i;
    for (i = 0; i < count; i++) {
        int error = 0;
        double value = te_interp(expressions[i], &error);
        printf("%-16s -> %.6f (err %d)\n", expressions[i], value, error);
    }

    {
        double x = 6.0, y = 7.0;
        te_variable variables[] = { { "x", &x }, { "y", &y } };
        int error = 0;
        te_expr *compiled = te_compile("x*y+1", variables, 2, &error);
        if (compiled) {
            printf("compiled x*y+1 -> %.1f (err %d)\n", te_eval(compiled), error);
            x = 2.0;
            printf("with x=2 -> %.1f\n", te_eval(compiled));
            te_free(compiled);
        } else {
            printf("compiled x*y+1 -> failed (err %d)\n", error);
        }
    }
    return 0;
}
