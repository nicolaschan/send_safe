use std::{any::Any, sync::{Arc, Mutex}};
use crossbeam::channel::{Receiver, RecvError, Sender, unbounded};
use thiserror::Error;

#[cfg(test)]
mod tests {
    use std::{sync::{Arc, Mutex, mpsc::{Receiver, Sender, channel}}};

    use crate::{ExecutionError, SendWrapperThread};

    #[test]
    fn test_send_pointer() {
        let make_pointer = ||  {
            let my_box = Box::new(42);
            Box::into_raw(my_box)
        };
        let mut send_pointer = SendWrapperThread::new(make_pointer);

        let return_value_mutex: Arc<Mutex<Option<u128>>> = Arc::new(Mutex::new(None));
        let return_value_mutex_clone = return_value_mutex.clone();
        std::thread::spawn(move || {
            let return_value = send_pointer.execute(|inner| {
                let my_box = unsafe { Box::from_raw(inner) };
                (Some(inner), *my_box + 1)
            }).unwrap();

            let mut guard = return_value_mutex_clone.lock().unwrap(); 
            *guard = Some(return_value);
        }).join().unwrap();
        
        let return_value = return_value_mutex.lock().unwrap().take().unwrap();
        assert_eq!(43, return_value);
    }

    #[test]
    fn test_drop() {
        let (sender, receiver): (Sender<()>, Receiver<()>) = channel();
        {
            SendWrapperThread::new(move || receiver);
        }
        assert!(sender.send(()).is_err());
    }

    #[test]
    fn test_manually_dropped() {
        let (sender, receiver): (Sender<()>, Receiver<()>) = channel();
        let mut wrapper = SendWrapperThread::new(move || receiver);
        assert!(sender.send(()).is_ok());
        assert!(wrapper.is_alive());
        drop(wrapper);
        assert!(sender.send(()).is_err());
    }

    #[test]
    fn test_dropped_when_closure_consumes_value() {
        let (sender, receiver): (Sender<()>, Receiver<()>) = channel();
        let mut wrapper = SendWrapperThread::new(move || receiver);
        assert!(sender.send(()).is_ok());
        assert!(wrapper.is_alive());
        wrapper.execute(|_inner| (None, ())).unwrap();
        assert!(sender.send(()).is_err());
        assert!(wrapper.is_dead());
    }

    #[test]
    fn test_could_not_send_error() {
        let mut wrapper = SendWrapperThread::new(|| ());
        wrapper.execute(|_inner| (None, ())).unwrap();
        let result =  wrapper.execute(|_inner| (None, ()));
        println!("{:?}", result);
        assert!(matches!(result, Err(ExecutionError::CouldNotSendError(..))));
        format!("{:?}", result); // should implement debug
        format!("{:#?}", result); // should implement format
    }

    #[test]
    fn test_no_response_error() {
        let mut wrapper = SendWrapperThread::new(|| ());
        let result: Result<usize, ExecutionError> = wrapper.execute(|_inner| panic!("panic!"));
        assert!(matches!(result, Err(ExecutionError::NoResponseError(..))));
        format!("{:?}", result); // should implement debug
        format!("{:#?}", result); // should implement format
    }
}

#[derive(Error, Debug)]
pub enum ExecutionError {
    #[error("error sending function call instruction to wrapped value (maybe it is dead)")]
    CouldNotSendError(Box<dyn Any + 'static>),
    #[error("error receiving response from function call to wrapped value")]
    NoResponseError(RecvError),
}

type RemoteExecutorClosure<T> = dyn (FnOnce(T) -> (Option<T>, Box<dyn Any + Send>)) + Send;
struct SendCommand<T> {
    closure: Box<RemoteExecutorClosure<T>>
}

#[derive(Clone)]
pub struct SendWrapperThread<T: 'static> {
    lock: Arc<Mutex<()>>,
    sender: Sender<SendCommand<T>>,
    receiver: Receiver<Box<dyn Any + Send>>,
}

impl<T: 'static> SendWrapperThread<T> {
    pub fn new(make_inner: impl (FnOnce() -> T) + Send + 'static) -> SendWrapperThread<T> {
        let (inside_sender, outside_receiver) = unbounded();
        let (outside_sender, inside_receiver): (Sender<SendCommand<T>>, Receiver<SendCommand<T>>) = unbounded();
        std::thread::spawn(move || {
            let mut inner = make_inner();
            while let Ok(message) = inside_receiver.recv() {
                let closure= message.closure;
                let (inner_option, return_value) = closure(inner);

                // If we cannot send the return values out,
                // then we should panic because we cannot respond to function calls.
                inside_sender.send(return_value).expect("Failed to exfiltrate return values"); 

                match inner_option {
                    Some(new_inner) => {
                        inner = new_inner;
                    },
                    None => { break; },
                }
            }
        });
        SendWrapperThread {
            lock: Arc::new(Mutex::new(())),
            sender: outside_sender,
            receiver: outside_receiver,
        }
    }

    pub fn execute<U: Any + Send + 'static>(&mut self, closure: impl (FnOnce(T) -> (Option<T>, U)) + Send + 'static) -> Result<U, ExecutionError> {
        let wrapped_closure: Box<RemoteExecutorClosure<T>> = Box::new(|inner| {
            let (new_inner, return_value) = closure(inner);
            let return_value_casted = Box::new(return_value);
            (new_inner, return_value_casted)
        });
        let send_message = SendCommand {
            closure: wrapped_closure 
        };
        let guard = self.lock.lock();
        self.sender.send(send_message)
            .map_err(|send_error| ExecutionError::CouldNotSendError(Box::new(send_error)))?;
        let return_value = self.receiver.recv()
            .map_err(ExecutionError::NoResponseError)?;
        drop(guard);
        let  return_value  = return_value.downcast::<U>().unwrap();
        Ok(*return_value)
    }

    pub fn is_alive(&mut self) -> bool {
        self.execute(|inner| (Some(inner), ())).is_ok()
    }

    pub fn is_dead(&mut self) -> bool {
        !self.is_alive()
    }
}

impl<T: 'static> Drop for SendWrapperThread<T> {
    fn drop(&mut self) {
        let result = self.execute(|_inner| {
            (None, ())
        });
        if result.is_err() {
            // If we couldn't communicate, then it's probably already dropped
            // so do nothing.
        }
    }
}