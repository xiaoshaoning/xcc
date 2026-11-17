/* math.h - C99 7.12, the subset a program on a desktop C library actually calls. */
#ifndef XCC_MATH_H
#define XCC_MATH_H

#define HUGE_VAL 1.79769313486231570815e+308

double acos(double x);
double asin(double x);
double atan(double x);
double atan2(double y, double x);
double cos(double x);
double sin(double x);
double tan(double x);
double cosh(double x);
double sinh(double x);
double tanh(double x);
double exp(double x);
double frexp(double value, int *exponent);
double ldexp(double value, int exponent);
double log(double x);
double log10(double x);
double modf(double value, double *integral);
double pow(double base, double exponent);
double sqrt(double x);
double ceil(double x);
double fabs(double x);
double floor(double x);
double fmod(double numerator, double denominator);

#endif
