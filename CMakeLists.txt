cmake_minimum_required(VERSION 3.26)

# The version is read from here by the Python build too (pyproject.toml).
project(layeredconf VERSION 0.1.0 LANGUAGES CXX)

option(LAYEREDCONF_BUILD_TESTS "Build the C++ tests" OFF)
option(LAYEREDCONF_BUILD_PYTHON "Build the Python module" OFF)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

find_package(tomlplusplus 3.4 REQUIRED)

add_library(layeredconf STATIC src/config.cpp src/envfile.cpp src/text.cpp)
target_include_directories(layeredconf PUBLIC $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>)
target_link_libraries(layeredconf PRIVATE tomlplusplus::tomlplusplus)
if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
  target_compile_options(layeredconf PRIVATE -Wall -Wextra -Wpedantic)
endif()

if(LAYEREDCONF_BUILD_TESTS)
  enable_testing()
  find_package(doctest REQUIRED)
  add_executable(layeredconf_tests tests/cpp/main.cpp tests/cpp/test_text.cpp tests/cpp/test_envfile.cpp
                                   tests/cpp/test_config.cpp)
  target_include_directories(layeredconf_tests PRIVATE src)
  target_link_libraries(layeredconf_tests PRIVATE layeredconf doctest::doctest)
  add_test(NAME layeredconf_tests COMMAND layeredconf_tests)
endif()

if(LAYEREDCONF_BUILD_PYTHON)
  find_package(Python 3.10 REQUIRED COMPONENTS Interpreter Development.Module)
  find_package(pybind11 CONFIG REQUIRED)
  pybind11_add_module(_core MODULE python/bindings.cpp)
  target_link_libraries(_core PRIVATE layeredconf)
  target_compile_definitions(_core PRIVATE LAYEREDCONF_VERSION="${PROJECT_VERSION}")
  # The wheel brings its C++ runtime along: it must not depend on the libstdc++ of the build environment.
  if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
    target_link_options(_core PRIVATE -static-libstdc++ -static-libgcc)
  endif()
  install(TARGETS _core DESTINATION layeredconf)
endif()
