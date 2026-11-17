/* Structs: passed by value, assigned, and a member read through a pointer. */
struct point {
    int x;
    int y;
};

struct rect {
    struct point top_left;
    struct point size;
};

int point_sum(struct point p) {
    return p.x + p.y;
}

struct point make_point(int x, int y) {
    struct point p;
    p.x = x;
    p.y = y;
    return p;
}

int rect_area(struct rect *r) {
    return r->size.x * r->size.y;
}

int copy_and_change(struct point p) {
    struct point q;
    q = p;
    q.x = q.x + 100;
    return q.x + q.y;
}

int rect_total(struct rect r) {
    return r.top_left.x + r.top_left.y + r.size.x + r.size.y;
}
