cmake_minimum_required(VERSION 3.14)
project(AETHER64_Benchmarks C CXX)

set(CMAKE_C_STANDARD 11)
set(CMAKE_CXX_STANDARD 17)

# Benchmark test harnesses
add_executable(test_mba tests/test_mba.c)
add_executable(test_cff tests/test_cff.c)

enable_testing()
add_test(NAME TestMBA COMMAND test_mba)
add_test(NAME TestCFF COMMAND test_cff)
