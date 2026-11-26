-- Exercises the whole language, printing everything, so two builds can be diffed.
local out = {}
local function say(...)
  local parts = {}
  for i = 1, select('#', ...) do
    parts[i] = tostring((select(i, ...)))
  end
  out[#out + 1] = table.concat(parts, " ")
end

-- integers, floats, division, bitwise
say(7 // 2, 7 % 2, 7 / 2, 2^10, 1 << 10, 0xFF ~ 0x0F, -1 >> 1)
say(math.maxinteger, math.mininteger, math.maxinteger + 1 == math.mininteger)
say(1/0, -1/0, 0/0 ~= 0/0, math.type(1), math.type(1.0))
say(string.format("%d %5.2f %g %x %q", 42, 3.14159, 1e300, 255, 'a"b'))

-- strings and patterns
local s = "the quick brown fox"
say(s:upper() ~= nil, s:sub(5, 9), #s, s:find("quick"), s:rep(2, "-"))
say(s:gsub("%w+", function(w) return w:sub(1, 1) end))
for k, v in s:gmatch("(%a+)=(%d+)") do say(k, v) end
say(s:match("(%a+) (%a+)"))
say(("x"):byte(), string.char(120, 121, 122))
say(string.pack("i4i4", 1, 2):byte(1, -1))
say(string.unpack("i4i4", string.pack("i4i4", 7, 8)))

-- tables, sorting, metamethods
local t = { 5, 2, 9, 1 }
table.sort(t)
say(table.concat(t, ","), #t, t[1], t[#t])
table.sort(t, function(a, b) return a > b end)
say(table.concat(t, ","))
local set = {}
for _, v in ipairs({ 1, 2, 2, 3 }) do set[v] = (set[v] or 0) + 1 end
local keys = {}
for k in pairs(set) do keys[#keys + 1] = k end
table.sort(keys)
say(table.concat(keys, ","), set[2])
say(table.unpack({ 10, 20, 30 }))

local point = {}
point.__index = point
point.__add = function(a, b) return setmetatable({ x = a.x + b.x }, point) end
point.__tostring = function(p) return "(" .. p.x .. ")" end
point.__eq = function(a, b) return a.x == b.x end
point.__len = function(p) return p.x end
local p1 = setmetatable({ x = 1 }, point)
local p2 = setmetatable({ x = 2 }, point)
say(tostring(p1 + p2), p1 == p2, #p1)

-- closures, upvalues, varargs, recursion
local function counter()
  local n = 0
  return function() n = n + 1 return n end
end
local c = counter()
c() c()
say(c())
local function sum(...)
  local total = 0
  for i = 1, select("#", ...) do total = total + select(i, ...) end
  return total, select("#", ...)
end
say(sum(1, 2, 3, 4, 5))
local function fib(n) if n < 2 then return n end return fib(n - 1) + fib(n - 2) end
say(fib(20))

-- coroutines
local co = coroutine.create(function(a)
  local b = coroutine.yield(a + 1)
  local d = coroutine.yield(b * 2)
  return d + 100
end)
say(coroutine.resume(co, 1))
say(coroutine.resume(co, 10))
say(coroutine.resume(co, 5))
say(coroutine.status(co))

-- errors, pcall, xpcall, error objects
local ok, err = pcall(function() error({ code = 42 }) end)
say(ok, type(err), err.code)
-- the location a runtime error carries is the file's name, which a test must not depend on
say(select(1, pcall(function() return 1 // 0 end)),
    (select(2, pcall(function() return 1 // 0 end)):gsub("^.-: ", "")))
say(select(2, xpcall(function() error("boom", 0) end, function(e) return "handled: " .. e end)))

-- goto, loops, repeat, numeric for with steps
do
  local i, total = 1, 0
  ::again::
  total = total + i
  i = i + 1
  if i <= 5 then goto again end
  say("goto", total)
end
local n = 0
repeat n = n + 1 until n >= 3
say("repeat", n)
for i = 10, 1, -3 do io.write(i, " ") end
io.write("\n")

-- string library corners, utf8, os
say(("abc"):byte(1, -1))
say(utf8.char(72, 0x65, 0x6C, 0x6C, 0x6F), utf8.len("héllo"), utf8.codepoint("é"))
say(tostring(123), tostring(1.5), tonumber("0x10"), tonumber("  12  "), tonumber("1e3"))
say(math.floor(-3.5), math.ceil(-3.5), math.fmod(7, 3), math.abs(math.mininteger))
say(string.format("%.3s|%-4d|%+.1e", "abcdef", 7, 1234.5))

-- io and os round trip
local name = os.tmpname()
local f = assert(io.open(name, "w"))
f:write("line one\n", 42, "\n")
f:close()
local r = assert(io.open(name, "r"))
local a, b = r:read("l"), r:read("n")
r:close()
os.remove(name)
say(a, b)

-- ldump/lundump, through the interpreter: bytecode written by string.dump and read back by load
local dumped = string.dump(function(n) return n * 2 end)
say(#dumped > 20, load(dumped)(21))

-- load, dofile-less compilation, and errors from load
say(load("return 1 + 2")())
say(load("syntax error here") == nil)

-- metatables on strings through the string library, and __index chains
local base = { greeting = "hi" }
local derived = setmetatable({}, { __index = base })
say(derived.greeting, getmetatable(derived).__index == base)

print(table.concat(out, "\n"))
