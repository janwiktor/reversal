// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The priority of the threads that share out a piece of work.
//!
//! On Apple's systems a thread has a quality-of-service class, which sets its priority and the
//! cores it may use. A thread started the usual way gets the default class, whatever class its
//! starter has. A starter of a higher class that then waits for it is held up by anything that
//! runs between the two: a priority inversion. So a thread that works for another takes that
//! thread's class. Elsewhere threads start with their starter's priority, and this does nothing.

/// A thread's priority, to hand to the threads that work for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Priority(u32);

/// `QOS_CLASS_USER_INITIATED`: work the user asked for and waits to see.
#[cfg(target_vendor = "apple")]
const USER_INITIATED: u32 = 0x19;

#[cfg(target_vendor = "apple")]
extern "C" {
    // Both are in the system library, declared in <pthread/qos.h>.
    fn qos_class_self() -> u32;
    fn pthread_set_qos_class_self_np(
        qos_class: u32,
        relative_priority: std::ffi::c_int,
    ) -> std::ffi::c_int;
}

impl Priority {
    /// The calling thread's. A program's first thread has the class kept for drawing and input,
    /// which calculations shouldn't compete with, so that counts as the one below it.
    pub fn current() -> Self {
        #[cfg(target_vendor = "apple")]
        // SAFETY: takes nothing and reads the calling thread's own class.
        return Self(unsafe { qos_class_self() }.min(USER_INITIATED));
        #[cfg(not(target_vendor = "apple"))]
        Self(0)
    }

    /// Gives it to the calling thread. Leaves a thread as it is where the system has no classes,
    /// or the thread it came from had none.
    pub fn adopt(self) {
        #[cfg(target_vendor = "apple")]
        if self.0 != 0 {
            // SAFETY: changes the calling thread only; a class the system refuses changes nothing.
            unsafe { pthread_set_qos_class_self_np(self.0, 0) };
        }
    }
}

#[cfg(all(test, target_vendor = "apple"))]
mod tests {
    use super::*;

    /// `QOS_CLASS_UTILITY`: long work that the user doesn't wait for.
    const UTILITY: Priority = Priority(0x11);

    #[test]
    fn a_thread_takes_the_priority_it_is_given() {
        let seen = std::thread::spawn(|| {
            UTILITY.adopt();
            Priority::current()
        });
        assert_eq!(seen.join().unwrap(), UTILITY);
    }

    #[test]
    fn a_thread_started_the_usual_way_has_another_priority_than_its_starter() {
        // What `adopt` is for: without it the class isn't passed on.
        let seen = std::thread::spawn(|| {
            UTILITY.adopt();
            std::thread::spawn(Priority::current).join().unwrap()
        });
        assert_ne!(seen.join().unwrap(), UTILITY);
    }

    #[test]
    fn shared_out_work_runs_at_the_priority_of_the_thread_that_waits_for_it() {
        let seen = std::thread::spawn(|| {
            UTILITY.adopt();
            // Enough for a part per core.
            let mut classes = vec![0u32; 1 << 20];
            crate::image::for_each_part(&mut classes, 1, |_, part| {
                part.fill(Priority::current().0)
            });
            classes
        });
        assert!(seen.join().unwrap().iter().all(|&class| class == UTILITY.0));
    }

    #[test]
    fn the_class_for_drawing_and_input_is_not_passed_on() {
        let seen = std::thread::spawn(|| {
            // QOS_CLASS_USER_INTERACTIVE.
            Priority(0x21).adopt();
            Priority::current()
        });
        assert_eq!(seen.join().unwrap(), Priority(USER_INITIATED));
    }
}
